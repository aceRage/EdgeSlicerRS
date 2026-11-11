//! Ironing for every 3MF the reference slicer ships or tests with: every
//! solid layer of every project is ironed, so that whatever shapes they
//! hold go through the area selection and the lines.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::Project;
use edge_clip::Join;
use edge_config::Value;
use edge_extrusion::Role;
use edge_geom::{ExPolygon, Point2, scaled};
use edge_infill::{InfillSettings, IronedLayer, IroningSettings};
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

#[test]
fn every_solid_layer_is_ironed_inside_its_outline() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();

    let mut failures = Vec::new();
    let (mut ironed_projects, mut lines) = (0, 0);
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
                filament: body.filament,
            })
            .collect();
        let mut settings = project.settings.unwrap_or_default();
        settings.insert("ironing_type", Value::One("solid".to_owned()));
        // Lines further apart than a real print's, to keep the test short.
        settings.insert("ironing_spacing", Value::One("0.4".to_owned()));
        let walls = WallSettings::from_settings(&settings).unwrap();
        let infill = InfillSettings::from_settings(&settings).unwrap();
        let ironing = IroningSettings::from_settings(&settings)
            .unwrap()
            .for_filament(1, 0.2);

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
        let inside_all =
            edge_infill::inside_walls(std::slice::from_ref(&inside), &slices, None).unwrap();
        let shells = infill.shells(0.2).unwrap();
        let surfaces =
            edge_infill::classify(&slices, &slices, &inside, &inside_all, &shells).unwrap();
        let faces = edge_infill::exposed(&slices, &slices, shells.opening).unwrap();
        ironed_projects += 1;

        let mut problems = Vec::new();
        let last = layers.len() - 1;
        for index in 0..layers.len() {
            let areas = edge_infill::ironing_areas(
                &IronedLayer {
                    slices: slices[index],
                    faces: &faces[index],
                    on_bed: index == 0,
                    surfaces: &surfaces[index],
                    infill: inside[index],
                    outline: slices[index],
                    ends_here: &|_| index == last,
                },
                &ironing,
            )
            .unwrap();
            // What is ironed lies inside the layer, half a nozzle from
            // its edge.
            let within = edge_clip::offset(slices[index], -scaled(0.199), Join::MITER).unwrap();
            let outside: f64 = edge_clip::difference(&areas, &within)
                .unwrap()
                .iter()
                .map(ExPolygon::area_mm2)
                .sum();
            if outside > 1e-3 {
                problems.push(format!(
                    "layer {index}: {outside} mm² ironed nearer the edge than the inset"
                ));
            }
            // The last layer faces up, all of it: all of it is ironed.
            let area = |regions: &[ExPolygon]| regions.iter().map(ExPolygon::area_mm2).sum::<f64>();
            if index == last
                && index > 0
                && (area(&areas) - area(&within)).abs() > 0.02 * area(&within).max(1.0)
            {
                problems.push(format!(
                    "layer {index}: the last layer has {} mm² ironed of {}",
                    area(&areas),
                    area(&within)
                ));
            }
            let origin = Point2::from_mm(0.0, 0.0);
            let made = edge_infill::iron(&areas, index, &ironing, &|_| origin, 1).unwrap();
            lines += made.len();
            if made
                .iter()
                .any(|line| line.points.len() < 2 || line.role != Role::Ironing)
            {
                problems.push(format!("layer {index}: an ironing line is degenerate"));
            }
            // Lines 0.4 mm apart lay about their area over 0.4 in
            // length; small and ragged areas less, never much more.
            let length: f64 = made
                .iter()
                .map(edge_extrusion::ExtrusionPath::length_mm)
                .sum();
            let expected = area(&areas) / 0.4;
            if length > 1.15 * expected + 5.0 {
                problems.push(format!(
                    "layer {index}: {length} mm of ironing over {} mm²",
                    area(&areas)
                ));
            }
            // No line strays from the area it irons.
            let reach = edge_clip::offset(&areas, scaled(0.01), Join::MITER).unwrap();
            let stray: Vec<Vec<Point2>> = made.iter().map(|line| line.points.clone()).collect();
            let off: f64 = edge_clip::lines_outside(&stray, &reach)
                .unwrap()
                .iter()
                .flat_map(|piece| piece.windows(2))
                .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
                .sum();
            if off > scaled(0.05) as f64 {
                problems.push(format!("layer {index}: ironing leaves its area"));
            }
        }
        if !problems.is_empty() {
            problems.truncate(4);
            failures.push(format!("{}: {}", path.display(), problems.join("; ")));
        }
    }
    assert!(
        ironed_projects >= 10 && lines > 1000,
        "only {ironed_projects} projects and {lines} lines"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
