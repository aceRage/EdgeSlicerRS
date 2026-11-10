//! G-code for every 3MF the reference slicer ships or tests with, read
//! back to check it is well formed and adds up.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::Project;
use edge_gcode::{Layer, PrintSettings};
use edge_geom::ExPolygon;
use edge_infill::InfillSettings;
use edge_perimeter::WallSettings;
use edge_slice::Body;

fn reference() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer");
    assert!(
        dir.join("tests").is_dir(),
        "reference slicer missing; run `git submodule update --init`"
    );
    dir
}

fn projects_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            projects_under(&path, found);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
        {
            found.push(path);
        }
    }
}

/// Checks one line against the small set of forms the writer may produce.
fn malformed(line: &str) -> bool {
    if line.starts_with(';') {
        return false;
    }
    let mut words = line.split(' ');
    match words.next() {
        Some("G1") => !words.all(|word| {
            let mut chars = word.chars();
            matches!(chars.next(), Some('X' | 'Y' | 'Z' | 'E' | 'F'))
                && chars.as_str().parse::<f64>().is_ok_and(f64::is_finite)
        }),
        Some("G28" | "G21" | "G90" | "M83" | "M84") => words.next().is_some(),
        Some("G92") => line != "G92 E0",
        Some("M104" | "M109" | "M140" | "M190") => !words
            .next()
            .and_then(|word| word.strip_prefix('S'))
            .is_some_and(|value| value.parse::<u32>().is_ok()),
        _ => true,
    }
}

#[test]
fn every_reference_project_exports_well_formed_gcode_that_adds_up() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();

    let mut failures = Vec::new();
    let (mut exported, mut bytes) = (0, 0);
    for path in &paths {
        let project = Project::open(path).unwrap();
        let bodies = project.bodies(None);
        let Some((min, max)) = project.mesh(None).bounds() else {
            continue;
        };
        let bodies: Vec<Body<'_>> = bodies
            .iter()
            .map(|body| Body {
                solid: &body.solid,
                negative: body.negative.as_ref(),
            })
            .collect();
        let settings = project.settings.unwrap_or_default();
        let walls = WallSettings::from_settings(&settings).unwrap();
        let infill = InfillSettings::from_settings(&settings).unwrap();
        let print = PrintSettings::from_settings(&settings).unwrap();

        let planes = edge_slice::layer_planes(min[2].into(), max[2].into(), 0.2, 0.2);
        let layers = edge_slice::slice_regions(&bodies, &planes, 49_000).unwrap();
        let perimeters: Vec<_> = layers
            .iter()
            .enumerate()
            .map(|(index, layer)| {
                edge_perimeter::classic(
                    &layer.regions,
                    &walls.params_at(0.2, index, layers.len()).unwrap(),
                )
                .unwrap()
            })
            .collect();
        let slices: Vec<&[ExPolygon]> = layers
            .iter()
            .map(|layer| layer.regions.as_slice())
            .collect();
        let inside: Vec<&[ExPolygon]> = perimeters
            .iter()
            .map(|layer| layer.infill.as_slice())
            .collect();
        let surfaces =
            edge_infill::classify(&slices, &slices, &inside, &infill.shells(0.2).unwrap()).unwrap();
        let fills: Vec<_> = surfaces
            .iter()
            .enumerate()
            .map(|(index, surfaces)| {
                edge_infill::fill(surfaces, index, &infill.params(0.2, index == 0).unwrap())
                    .unwrap()
            })
            .collect();
        let plan: Vec<Layer<'_>> = (0..layers.len())
            .map(|index| Layer {
                z: 0.2 * (index + 1) as f64,
                height: 0.2,
                loops: &perimeters[index].loops,
                islands: &perimeters[index].islands,
                paths: &fills[index],
                infill_area: &perimeters[index].infill,
            })
            .collect();

        let mut out = Vec::new();
        let summary = edge_gcode::write(&mut out, &plan, &print).unwrap();
        let text = String::from_utf8(out).unwrap();
        exported += 1;
        bytes += text.len();

        let mut problems = Vec::new();
        if let Some(line) = text.lines().find(|line| malformed(line)) {
            problems.push(format!("malformed line: {line}"));
        }
        let word = |line: &str, letter: char| -> Option<f64> {
            line.split(' ')
                .find_map(|part| part.strip_prefix(letter)?.parse().ok())
        };
        // Everything the planner was given is in the file, and nothing else.
        let planned: f64 = perimeters
            .iter()
            .flat_map(|layer| &layer.loops)
            .map(|wall| wall.volume_mm3())
            .sum::<f64>()
            + fills
                .iter()
                .flatten()
                .map(|line| line.volume_mm3())
                .sum::<f64>();
        let expected = planned * print.filament_flow_ratio * print.print_flow_ratio;
        if (summary.filament_mm3 - expected).abs() > 1e-6 * expected.max(1.0) {
            problems.push(format!(
                "{} mm³ exported of {expected} planned",
                summary.filament_mm3
            ));
        }
        // Summed from the text; each move is rounded to 1e-5 mm.
        let moves = || text.lines().filter(|line| line.starts_with("G1 "));
        let fed: f64 = moves()
            .filter(|line| line.contains(" X"))
            .filter_map(|line| word(line, 'E'))
            .sum();
        if (fed - summary.filament_mm).abs() > 1e-5 * moves().count() as f64 {
            problems.push(format!(
                "file feeds {fed} mm, summary says {}",
                summary.filament_mm
            ));
        }
        // Retractions and their undoing cancel out, bar the final one.
        let net: f64 = moves()
            .filter(|line| !line.contains(" X"))
            .filter_map(|line| word(line, 'E'))
            .sum();
        let last = if summary.retractions > 0 {
            print.retraction_length
        } else {
            0.0
        };
        if (net + last).abs() > 1e-6 {
            problems.push(format!("retraction does not balance: net {net} mm"));
        }
        // The nozzle never drops below the layer it is printing.
        let mut layer_z = 0.0;
        for line in text.lines() {
            if let Some(z) = line.strip_prefix(";Z:") {
                layer_z = z.parse().unwrap();
            } else if line.starts_with("G1 ") && word(line, 'Z').is_some_and(|z| z < layer_z - 1e-9)
            {
                problems.push(format!("nozzle below the layer at {layer_z}: {line}"));
                break;
            }
        }
        if !problems.is_empty() {
            failures.push(format!("{}:\n  {}", path.display(), problems.join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        exported >= 25 && bytes > 1_000_000,
        "{exported} projects exported, {bytes} bytes"
    );
}
