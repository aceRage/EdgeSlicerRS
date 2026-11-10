//! Slices one project with the reference C++ slicer and with this engine,
//! then compares how long each took and what each printed.

mod gcode;

use std::{
    collections::BTreeSet,
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode, Stdio},
    time::{Duration, Instant},
};

use clap::Parser;
use gcode::{Layer, Print, Stat};

type Failure = Box<dyn Error>;

#[derive(Parser)]
#[command(
    name = "edge-parity",
    version,
    about = "Compare this engine with the reference slicer on one project"
)]
struct Cli {
    /// 3MF project carrying its own settings.
    project: PathBuf,
    /// The reference slicer's executable, e.g.
    /// EdgeSlicer.app/Contents/MacOS/EdgeSlicer.
    #[arg(long, env = "EDGE_REFERENCE_BIN")]
    reference: PathBuf,
    /// This engine's `edge` executable; defaults to the one beside this tool.
    #[arg(long)]
    engine: Option<PathBuf>,
    /// Plate to slice.
    #[arg(long, default_value_t = 1)]
    plate: u32,
    /// Times each slicer is run; the fastest run is reported.
    #[arg(long, default_value_t = 3)]
    runs: usize,
    /// Keep both G-code files and the reference's log in this directory.
    #[arg(long)]
    keep: Option<PathBuf>,
    /// Draw one layer of both toolpaths over each other in this SVG file.
    #[arg(long)]
    svg: Option<PathBuf>,
    /// Layer for --svg, counting from 0; defaults to the middle layer.
    #[arg(long, requires = "svg")]
    svg_layer: Option<usize>,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Failure> {
    let engine = match &cli.engine {
        Some(path) => path.clone(),
        None => {
            let name = if cfg!(windows) { "edge.exe" } else { "edge" };
            std::env::current_exe()?.with_file_name(name)
        }
    };
    for (what, path) in [
        ("reference slicer", &cli.reference),
        ("engine", &engine),
        ("project", &cli.project),
    ] {
        if !path.is_file() {
            return Err(format!("{what} not found at {}", path.display()).into());
        }
    }
    let runs = cli.runs.max(1);
    let work = match &cli.keep {
        Some(dir) => dir.clone(),
        None => std::env::temp_dir().join(format!("edge-parity-{}", std::process::id())),
    };
    fs::create_dir_all(&work)?;
    let result = compare(&cli, &engine, &work, runs);
    if cli.keep.is_none() {
        // Best effort: a leftover temporary directory is not a failed comparison.
        let _ = fs::remove_dir_all(&work);
    }
    result
}

fn compare(cli: &Cli, engine: &Path, work: &Path, runs: usize) -> Result<(), Failure> {
    // The reference is run from the work directory, so every path it is
    // given has to hold from there.
    let project = std::path::absolute(&cli.project)?;
    let work = &std::path::absolute(work)?;
    // The reference gets its own empty data directory so it neither reads
    // nor disturbs the user's, and is told to leave the hub alone.
    let reference_out = work.join("reference");
    let mut reference = Command::new(&cli.reference);
    // It drops a result.json wherever it is run from.
    reference.current_dir(work);
    reference
        .arg("--datadir")
        .arg(work.join("reference-data"))
        .args([
            "--no-hub",
            "--no-thumbnails",
            "--debug",
            "3",
            "--slice",
            &cli.plate.to_string(),
        ])
        .arg("--outputdir")
        .arg(&reference_out)
        .arg(&project);
    let reference_gcode = reference_out.join(format!("plate_{}.gcode", cli.plate));
    let (reference_time, reference_work) = fastest(
        &mut reference,
        runs,
        &work.join("reference.log"),
        &reference_gcode,
    )?;

    let engine_gcode = work.join("edge.gcode");
    let mut ours = Command::new(engine);
    ours.arg("slice")
        .arg(&project)
        .arg("--gcode")
        .arg(&engine_gcode);
    if cli.plate != 1 {
        ours.args(["--plate", &cli.plate.to_string()]);
    }
    let (engine_time, _) = fastest(&mut ours, runs, &work.join("edge.log"), &engine_gcode)?;

    let reference = gcode::parse(&fs::read_to_string(&reference_gcode)?, None);
    let ours = gcode::parse(&fs::read_to_string(&engine_gcode)?, None);
    let times = Times {
        runs,
        reference: reference_time,
        reference_work,
        engine: engine_time,
    };
    print!("{}", report(&cli.project, &times, &reference, &ours));

    if let Some(path) = &cli.svg {
        let layer = cli
            .svg_layer
            .unwrap_or(reference.layers.len().min(ours.layers.len()) / 2);
        let reference = gcode::parse(&fs::read_to_string(&reference_gcode)?, Some(layer));
        let ours = gcode::parse(&fs::read_to_string(&engine_gcode)?, Some(layer));
        let (Some(reference), Some(ours)) = (reference.layers.get(layer), ours.layers.get(layer))
        else {
            return Err(format!("--svg-layer {layer} is not in both prints").into());
        };
        fs::write(path, overlay(reference, ours))?;
        println!(
            "\nlayer {layer} drawn to {} (reference red, this engine blue)",
            path.display()
        );
    }
    if cli.keep.is_some() {
        println!("\noutputs kept in {}", work.display());
    }
    Ok(())
}

/// Runs a slicer `runs` times and returns its fastest wall time and, when
/// its log is timestamped, the shortest span the log covers. Its output
/// goes to `log`; `produces` must exist afterwards.
fn fastest(
    command: &mut Command,
    runs: usize,
    log: &Path,
    produces: &Path,
) -> Result<(Duration, Option<Duration>), Failure> {
    let mut best = Duration::MAX;
    let mut best_logged: Option<Duration> = None;
    for _ in 0..runs {
        let _ = fs::remove_file(produces);
        let output = fs::File::create(log)?;
        let started = Instant::now();
        let status = command
            .stdin(Stdio::null())
            .stdout(output.try_clone()?)
            .stderr(output)
            .status()?;
        best = best.min(started.elapsed());
        if !status.success() || !produces.is_file() {
            let tail: Vec<String> = fs::read_to_string(log)
                .unwrap_or_default()
                .lines()
                .rev()
                .take(8)
                .map(String::from)
                .collect();
            let tail: Vec<&str> = tail.iter().rev().map(String::as_str).collect();
            return Err(format!(
                "{} {status} without writing {}:\n{}",
                command.get_program().to_string_lossy(),
                produces.display(),
                tail.join("\n")
            )
            .into());
        }
        if let Some(logged) = logged_span(&fs::read_to_string(log).unwrap_or_default()) {
            best_logged = Some(best_logged.map_or(logged, |best| best.min(logged)));
        }
    }
    Ok((best, best_logged))
}

/// Fastest wall times over `runs` runs of each program.
struct Times {
    runs: usize,
    /// The reference from start to exit.
    reference: Duration,
    /// From the first line of the reference's log to its last. The program
    /// spends about two seconds starting and exiting whatever it is asked
    /// to do, even to print its help, so this is the fairer figure for the
    /// work of loading, slicing and exporting.
    reference_work: Option<Duration>,
    /// This engine from start to exit.
    engine: Duration,
}

/// Time between the first and last timestamped lines of the reference's
/// log, which look like `[2026-11-09 19:44:32.795889] ...`.
fn logged_span(log: &str) -> Option<Duration> {
    let seconds = |line: &str| -> Option<f64> {
        let clock = line
            .strip_prefix('[')?
            .split(']')
            .next()?
            .split(' ')
            .nth(1)?;
        let mut parts = clock.split(':').map(|part| part.parse::<f64>().ok());
        Some(parts.next()?? * 3600.0 + parts.next()?? * 60.0 + parts.next()??)
    };
    let mut stamps = log.lines().filter_map(seconds);
    let first = stamps.next()?;
    let last = stamps.next_back().unwrap_or(first);
    // A run that crosses midnight wraps round.
    let span = if last < first {
        last + 86_400.0 - first
    } else {
        last - first
    };
    Some(Duration::from_secs_f64(span))
}

/// Filament cross-section for the usual 1.75 mm filament, in mm².
const FILAMENT_AREA: f64 = std::f64::consts::PI * 0.875 * 0.875;

fn report(project: &Path, times: &Times, reference: &Print, ours: &Print) -> String {
    let mut out = String::new();
    let percent = |ours: f64, reference: f64| {
        if reference == 0.0 {
            "n/a".to_owned()
        } else {
            format!("{:+.1}%", (ours / reference - 1.0) * 100.0)
        }
    };
    let mut row = |label: &str, reference: String, ours: String, change: String| {
        writeln!(out, "{label:<24}{reference:>14}{ours:>14}{change:>16}").unwrap();
    };

    row(
        &project.file_name().unwrap_or_default().to_string_lossy(),
        "reference".into(),
        "this engine".into(),
        "change".into(),
    );
    let faster = |reference: f64, ours: f64| format!("{:.1}x faster", reference / ours.max(1e-6));
    let (rt, ot) = (times.reference.as_secs_f64(), times.engine.as_secs_f64());
    row(
        &format!("whole run, best of {}", times.runs),
        format!("{rt:.3} s"),
        format!("{ot:.3} s"),
        faster(rt, ot),
    );
    // The reference's own log brackets its real work; this engine has no
    // startup to speak of, so its whole run stands against that.
    if let Some(work) = times.reference_work {
        let work = work.as_secs_f64();
        row(
            "load, slice, export",
            format!("{work:.3} s"),
            format!("{ot:.3} s"),
            faster(work, ot),
        );
    }
    row(
        "layers",
        reference.layers.len().to_string(),
        ours.layers.len().to_string(),
        String::new(),
    );
    let (rf, of) = (reference.total(), ours.total());
    row(
        "filament",
        format!("{:.1} mm", rf.filament_mm),
        format!("{:.1} mm", of.filament_mm),
        percent(of.filament_mm, rf.filament_mm),
    );
    row(
        "  as volume",
        format!("{:.2} cm3", rf.filament_mm * FILAMENT_AREA / 1e3),
        format!("{:.2} cm3", of.filament_mm * FILAMENT_AREA / 1e3),
        String::new(),
    );
    row(
        "extruded path",
        format!("{:.2} m", rf.length_mm / 1e3),
        format!("{:.2} m", of.length_mm / 1e3),
        percent(of.length_mm, rf.length_mm),
    );
    row(
        "travel",
        format!("{:.2} m", reference.travel_mm / 1e3),
        format!("{:.2} m", ours.travel_mm / 1e3),
        percent(ours.travel_mm, reference.travel_mm),
    );
    row(
        "retractions",
        reference.retractions.to_string(),
        ours.retractions.to_string(),
        percent(ours.retractions as f64, reference.retractions as f64),
    );

    writeln!(out, "\nextruded path by feature, m").unwrap();
    let (reference_features, our_features) = (reference.features(), ours.features());
    let names: BTreeSet<&String> = reference_features
        .keys()
        .chain(our_features.keys())
        .collect();
    for name in names {
        let length = |features: &std::collections::BTreeMap<String, Stat>| {
            features.get(name).map_or(0.0, |s| s.length_mm)
        };
        let (r, o) = (length(&reference_features), length(&our_features));
        let change = if r == 0.0 {
            "only here".to_owned()
        } else if o == 0.0 {
            "missing".to_owned()
        } else {
            percent(o, r)
        };
        writeln!(
            out,
            "  {name:<22}{:>14.2}{:>14.2}{change:>16}",
            r / 1e3,
            o / 1e3
        )
        .unwrap();
    }

    // The machine's start G-code should come out the same from both: it is
    // the profile's template, and only the filling-in is ours.
    if !reference.start_block.is_empty() || !ours.start_block.is_empty() {
        let (r, o) = (&reference.start_block, &ours.start_block);
        let same = r.iter().zip(o).take_while(|(a, b)| a == b).count();
        if same == r.len() && same == o.len() {
            writeln!(out, "\nstart G-code: all {same} lines identical").unwrap();
        } else {
            writeln!(
                out,
                "\nstart G-code: first {same} lines identical, then (of {} and {} lines)",
                r.len(),
                o.len()
            )
            .unwrap();
            for index in same..(same + 6).min(r.len().max(o.len())) {
                let line = |block: &[String]| {
                    block
                        .get(index)
                        .map_or("(nothing)", String::as_str)
                        .to_owned()
                };
                writeln!(out, "  reference: {}\n  here:      {}", line(r), line(o)).unwrap();
            }
        }
    }

    // Layer by layer, where the two prints have the same layers.
    let pairs: Vec<(usize, &Layer, &Layer)> = reference
        .layers
        .iter()
        .zip(&ours.layers)
        .enumerate()
        .filter(|(_, (r, o))| (r.z - o.z).abs() < 1e-6)
        .map(|(index, (r, o))| (index, r, o))
        .collect();
    writeln!(
        out,
        "\n{} of {} reference layers have a layer at the same height here",
        pairs.len(),
        reference.layers.len()
    )
    .unwrap();
    // The reference files parts of a wall under other names where it
    // overhangs or sits on support, so walls are compared as a whole.
    let wall = |layer: &Layer| -> f64 {
        layer
            .features
            .iter()
            .filter(|(name, _)| name.to_lowercase().contains("wall"))
            .map(|(_, stat)| stat.length_mm)
            .sum()
    };
    let differences: Vec<(usize, f64)> = pairs
        .iter()
        .filter(|(_, r, _)| wall(r) > 0.0)
        .map(|(index, r, o)| (*index, (wall(o) / wall(r) - 1.0) * 100.0))
        .collect();
    if let Some(worst) = differences
        .iter()
        .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
    {
        let mean = differences.iter().map(|(_, d)| d.abs()).sum::<f64>() / differences.len() as f64;
        writeln!(
            out,
            "wall length per layer: differs by {mean:.2}% on average, most on layer {} ({:+.1}%)",
            worst.0, worst.1
        )
        .unwrap();
    }
    let shifts: Vec<(usize, f64)> = pairs
        .iter()
        .filter_map(|(index, r, o)| {
            let (r, o) = (r.bounds?, o.bounds?);
            Some((
                *index,
                (0..4).map(|i| (r[i] - o[i]).abs()).fold(0.0, f64::max),
            ))
        })
        .collect();
    if let Some(worst) = shifts.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        let mean = shifts.iter().map(|(_, d)| d).sum::<f64>() / shifts.len() as f64;
        writeln!(
            out,
            "extent of each layer's toolpaths: edges differ by {mean:.3} mm on average, most on layer {} ({:.3} mm)",
            worst.0, worst.1
        )
        .unwrap();
    }
    out
}

/// One layer of both prints, drawn over each other in mm with +y up.
fn overlay(reference: &Layer, ours: &Layer) -> String {
    let all = || {
        reference
            .segments
            .iter()
            .chain(&ours.segments)
            .flat_map(|segment| [segment.from, segment.to])
    };
    let (mut min, mut max) = ([f64::MAX; 2], [f64::MIN; 2]);
    for point in all() {
        for axis in 0..2 {
            min[axis] = min[axis].min(point[axis]);
            max[axis] = max[axis].max(point[axis]);
        }
    }
    if min[0] > max[0] {
        (min, max) = ([0.0; 2], [1.0; 2]);
    }
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    let margin = width.max(height) * 0.05;
    let mut body = String::new();
    for (segments, colour) in [
        (&reference.segments, "#d62828"),
        (&ours.segments, "#1d4ed8"),
    ] {
        write!(body, "<path stroke=\"{colour}\" d=\"").unwrap();
        for segment in segments {
            write!(
                body,
                "M{:.3} {:.3}L{:.3} {:.3}",
                segment.from[0], -segment.from[1], segment.to[0], -segment.to[1]
            )
            .unwrap();
        }
        body.push_str("\"/>");
    }
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{:.3} {:.3} {:.3} {:.3}\">\
         <g fill=\"none\" stroke-width=\"0.12\" stroke-opacity=\"0.7\" stroke-linecap=\"round\">{body}</g></svg>\n",
        min[0] - margin,
        -max[1] - margin,
        width + 2.0 * margin,
        height + 2.0 * margin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(text: &str) -> Print {
        gcode::parse(text, None)
    }

    #[test]
    fn report_compares_time_filament_features_and_layers() {
        let reference = print(
            ";LAYER_CHANGE\n;Z:0.2\nM83\nG1 X0 Y0\n;TYPE:Outer wall\nG1 X10 Y0 E1\n;TYPE:Overhang wall\nG1 X10 Y10 E1\n\
             ;LAYER_CHANGE\n;Z:0.4\n;TYPE:Outer wall\nG1 X0 Y10 E1\n",
        );
        let ours = print(
            ";LAYER_CHANGE\n;Z:0.2\nM83\nG1 X0 Y0\n;TYPE:Outer wall\nG1 X11 Y0 E1.1\n;TYPE:Sparse infill\nG1 X11 Y5 E0.4\n\
             ;LAYER_CHANGE\n;Z:0.5\n;TYPE:Outer wall\nG1 X0 Y5 E1\n",
        );
        let times = Times {
            runs: 3,
            reference: Duration::from_millis(2000),
            reference_work: Some(Duration::from_millis(400)),
            engine: Duration::from_millis(40),
        };
        let text = report(Path::new("dir/part.3mf"), &times, &reference, &ours);
        assert!(text.contains("part.3mf"), "{text}");
        assert!(text.contains("50.0x faster"), "{text}");
        // Against the 0.4 s the reference's log says it worked for.
        assert!(text.contains("10.0x faster"), "{text}");
        // 2.5 mm fed here against 3.0 in the reference.
        assert!(text.contains("-16.7%"), "{text}");
        let line = |name: &str| {
            text.lines()
                .find(|line| line.trim_start().starts_with(name))
                .unwrap()
                .to_owned()
        };
        assert!(line("Overhang wall").ends_with("missing"));
        assert!(line("Sparse infill").ends_with("only here"));
        // Only the first layers share a height. The reference's walls there
        // come to 20 mm under two names; this print's come to 11 mm.
        assert!(text.contains("1 of 2 reference layers"), "{text}");
        assert!(text.contains("most on layer 0 (-45.0%)"), "{text}");
        // The reference reaches y = 10 where this print stops at y = 5.
        assert!(text.contains("most on layer 0 (5.000 mm)"), "{text}");
    }

    #[test]
    fn logged_span_runs_from_first_stamp_to_last() {
        let log = "starting\n[2026-11-09 19:44:32.795889] [0x1] [info] a\nno stamp\n[2026-11-09 19:44:33.214038] [0x1] [info] b\n";
        let span = logged_span(log).unwrap().as_secs_f64();
        assert!((span - 0.418149).abs() < 1e-6, "{span}");
        let midnight = "[2026-11-09 23:59:59.500000] x\n[2026-11-10 00:00:01.000000] y\n";
        assert!((logged_span(midnight).unwrap().as_secs_f64() - 1.5).abs() < 1e-6);
        assert_eq!(logged_span("nothing timed"), None);
    }

    #[test]
    fn overlay_draws_both_prints() {
        let layer = |text: &str| gcode::parse(text, Some(0)).layers.remove(0);
        let reference = layer(";LAYER_CHANGE\nM83\nG1 X0 Y0\nG1 X10 Y0 E1\n");
        let ours = layer(";LAYER_CHANGE\nM83\nG1 X0 Y1\nG1 X10 Y1 E1\n");
        let svg = overlay(&reference, &ours);
        assert!(
            svg.contains("#d62828\" d=\"M0.000 -0.000L10.000 -0.000")
                && svg.contains("#1d4ed8\" d=\"M0.000 -1.000")
        );
    }
}
