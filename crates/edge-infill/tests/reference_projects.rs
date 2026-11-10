//! Surfaces and fill for every 3MF the reference slicer ships or tests with.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::Project;
use edge_extrusion::{ExtrusionPath, Role};
use edge_geom::ExPolygon;
use edge_infill::{InfillSettings, SurfaceKind};
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
fn surfaces_partition_the_infill_area_and_every_one_is_filled() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();

    let mut failures = Vec::new();
    let (mut filled, mut lines) = (0, 0);
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
        filled += 1;

        let mut problems = Vec::new();
        for (index, surfaces) in surfaces.iter().enumerate() {
            // Nothing is lost or counted twice when the area is split up.
            let whole: f64 = inside[index].iter().map(ExPolygon::area_mm2).sum();
            let parts: f64 = surfaces.iter().map(|surface| surface.area.area_mm2()).sum();
            if infill.sparse_infill_density_percent > 0.0
                && (whole - parts).abs() > 1e-4 * whole.max(1.0)
            {
                problems.push(format!(
                    "layer {index}: surfaces cover {parts} of {whole} mm²"
                ));
            }
            // The first layer rests on the bed and the last has nothing above.
            let all = |kind| surfaces.iter().all(|surface| surface.kind == kind);
            if (index == 0 && infill.bottom_shell_layers > 0 && !all(SurfaceKind::Bottom))
                || (index + 1 == layers.len()
                    && index > 0
                    && infill.top_shell_layers > 0
                    && !all(SurfaceKind::Top))
            {
                problems.push(format!(
                    "layer {index}: an end layer has surfaces of the wrong kind"
                ));
            }

            let params = infill.params(0.2, index == 0).unwrap();
            let fill = edge_infill::fill(surfaces, index, &params).unwrap();
            lines += fill.len();
            if fill.iter().any(|line| {
                line.points.len() != 2 || line.length_mm() < line.flow.width / 2.0 - 1e-6
            }) {
                problems.push(format!("layer {index}: a fill line is degenerate"));
            }
            // Solid surfaces get about their own area in plastic. Rows stop
            // short of the outline by up to a spacing, so the allowance
            // grows with how much outline there is.
            let solid = || {
                surfaces
                    .iter()
                    .filter(|s| s.kind == SurfaceKind::InternalSolid)
            };
            let solid_area: f64 = solid().map(|s| s.area.area_mm2()).sum();
            let outline: f64 = solid()
                .flat_map(|s| s.area.rings())
                .map(|ring| ring.perimeter_mm())
                .sum();
            let laid: f64 = fill
                .iter()
                .filter(|line| line.role == Role::SolidInfill)
                .map(|line: &ExtrusionPath| line.length_mm() * line.flow.spacing)
                .sum();
            if (laid - solid_area).abs() > 0.05 * solid_area + outline * params.solid.spacing {
                problems.push(format!(
                    "layer {index}: {laid} mm² of solid fill laid over {solid_area} mm²"
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
        filled >= 25 && lines > 100_000,
        "{filled} projects filled, {lines} lines"
    );
}
