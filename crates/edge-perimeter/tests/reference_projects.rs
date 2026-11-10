//! Walls for every 3MF the reference slicer ships or tests with.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::Project;
use edge_geom::ExPolygon;
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

fn area(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

#[test]
fn walls_are_well_formed_and_infill_stays_inside_on_every_reference_project() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();

    let mut failures = Vec::new();
    let (mut walled, mut loops) = (0, 0);
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
        // The project's own wall settings where it carries them.
        let walls = WallSettings::from_settings(&project.settings.unwrap_or_default()).unwrap();
        let planes = edge_slice::layer_planes(min[2].into(), max[2].into(), 0.2, 0.2);
        let layers = edge_slice::slice_regions(&bodies, &planes, 49_000).unwrap();
        walled += 1;

        let mut problems = Vec::new();
        for (index, layer) in layers.iter().enumerate() {
            let params = walls.params(0.2, index == 0).unwrap();
            let perimeters = match edge_perimeter::classic(&layer.regions, &params) {
                Ok(perimeters) => perimeters,
                Err(error) => {
                    problems.push(format!("layer {index}: {error}"));
                    continue;
                }
            };
            loops += perimeters.loops.len();
            for wall in &perimeters.loops {
                // Outer boundaries run counter-clockwise, hole walls clockwise.
                if wall.polygon.points.len() < 3 || wall.polygon.is_ccw() == wall.is_hole {
                    problems.push(format!(
                        "layer {index}: malformed loop at depth {}",
                        wall.depth
                    ));
                }
            }
            // Infill may reach into the walls but never outside the layer.
            let inside =
                area(&edge_clip::intersection(&perimeters.infill, &layer.regions).unwrap());
            let infill = area(&perimeters.infill);
            if (infill - inside).abs() > 1e-6 * infill.max(1.0) || infill > area(&layer.regions) {
                problems.push(format!(
                    "layer {index}: {infill} mm² of infill, {inside} of it inside the layer"
                ));
            }
        }
        if !problems.is_empty() {
            problems.truncate(3);
            failures.push(format!("{}:\n  {}", path.display(), problems.join("\n  ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(
        walled >= 25 && loops > 10_000,
        "{walled} projects walled, {loops} loops"
    );
}
