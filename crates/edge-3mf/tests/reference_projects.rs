//! Checks the reader against every 3MF the reference slicer ships or tests with.

use std::{
    fs,
    path::{Path, PathBuf},
};

use edge_3mf::Project;
use edge_mesh::Mesh;
use edge_slice::Body;

/// The reference's default `slice_closing_radius`, 0.049 mm, in nanometres.
const CLOSING_RADIUS: i64 = 49_000;

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
fn every_reference_project_opens_and_slices_into_closed_contours() {
    let mut paths = Vec::new();
    for dir in ["tests", "resources"] {
        projects_under(&reference().join(dir), &mut paths);
    }
    paths.sort();
    assert!(paths.len() >= 30, "only {} projects found", paths.len());

    let mut failures = Vec::new();
    let mut with_geometry = 0;
    for path in &paths {
        let project = match Project::open(path) {
            Ok(project) => project,
            Err(error) => {
                failures.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        let mesh = project.mesh(None);
        let Some((min, max)) = mesh.bounds() else {
            continue;
        };
        with_geometry += 1;
        if mesh.signed_volume() <= 0.0 {
            failures.push(format!(
                "{}: volume {} is not positive",
                path.display(),
                mesh.signed_volume()
            ));
        }

        let bodies = project.bodies(None);
        let bodies: Vec<Body<'_>> = bodies
            .iter()
            .map(|body| Body {
                solid: &body.solid,
                negative: body.negative.as_ref(),
                filament: body.filament,
            })
            .collect();
        let planes = edge_slice::layer_planes(min[2].into(), max[2].into(), 0.2, 0.2);
        let layers = match edge_slice::slice_regions(&bodies, &planes, CLOSING_RADIUS) {
            Ok(layers) => layers,
            Err(error) => {
                failures.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        let open: usize = layers.iter().map(|layer| layer.open_chains).sum();
        if open > 0 {
            failures.push(format!("{}: {open} open contours", path.display()));
        }
        // Regions are material: counter-clockwise outside, clockwise holes.
        let inverted = layers
            .iter()
            .flat_map(|layer| &layer.regions)
            .filter(|region| {
                !region.contour.is_ccw() || region.holes.iter().any(|hole| hole.is_ccw())
            })
            .count();
        if inverted > 0 || layers.iter().all(|layer| layer.regions.is_empty()) {
            failures.push(format!(
                "{}: {inverted} misoriented regions, or no regions at all",
                path.display()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Sliced-output files (*.gcode.3mf) and one settings-only fixture carry no models.
    assert!(
        with_geometry >= 25,
        "only {with_geometry} projects had geometry"
    );
}

#[test]
fn multi_plate_project_keeps_its_structure() {
    let project = Project::open(reference().join("tests/mcbrim_test.3mf")).unwrap();
    assert_eq!(project.objects.len(), 16);
    assert_eq!(
        project
            .plates
            .iter()
            .map(|plate| plate.id)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(
        project
            .objects
            .iter()
            .all(|object| object.name.as_deref() == Some("MC Arms") && object.parts.len() == 3)
    );

    let settings = project
        .settings
        .as_ref()
        .expect("the project carries its settings");
    assert_eq!(settings.text("printer_model"), Some("Snapmaker U1"));
    assert_eq!(settings.float("layer_height").unwrap(), Some(0.2));

    // Half of these objects are placed by mirroring transforms; each must
    // still come out as a solid rather than inside out.
    for object in &project.objects {
        for instance in &object.instances {
            let mut mesh = Mesh::default();
            for part in &object.parts {
                mesh.append(&part.mesh, &part.transform.then(&instance.transform));
            }
            assert!(
                mesh.signed_volume() > 0.0,
                "object {} is inside out",
                object.id
            );
        }
    }
    let on_plates: usize = project
        .plates
        .iter()
        .map(|plate| plate.instances.len())
        .sum();
    assert_eq!(on_plates, 16);
}

#[test]
fn plain_3mf_takes_names_from_model_settings() {
    let project =
        Project::open(reference().join("resources/handy_models/OrcaCube_v2.3mf")).unwrap();
    let names: Vec<_> = project
        .objects
        .iter()
        .map(|object| object.name.as_deref())
        .collect();
    assert_eq!(names, [Some("OrcaCube"), Some("OrcaPlug")]);
    assert!(project.plates.is_empty() && project.settings.is_none());
}

/// A project printed in several filaments says which part is printed in
/// which, and its bodies come one to a filament.
#[test]
fn parts_keep_their_filaments() {
    let project = Project::open(reference().join("tests").join("mcbrim_test.3mf")).unwrap();
    let object = &project.objects[0];
    let filaments: Vec<usize> = object.parts.iter().map(|part| part.filament).collect();
    assert_eq!(filaments.len(), 3);
    // Every part names a filament of its own, none the object's.
    assert!(
        filaments.iter().all(|filament| *filament > 1),
        "{filaments:?}"
    );
    let plate = project.plate_index(1);
    let bodies = project.bodies(plate);
    let used: Vec<usize> = bodies.iter().map(|body| body.filament).collect();
    assert_eq!(used, [2, 3, 4, 7]);
    assert!(bodies.iter().all(|body| !body.solid.triangles.is_empty()));
    // Together they are the same material as the plate's one mesh.
    let all: usize = bodies.iter().map(|body| body.solid.triangles.len()).sum();
    assert_eq!(all, project.mesh(plate).triangles.len());
}

#[test]
fn painted_colours_are_read_with_the_objects_they_are_on() {
    let project = Project::open(reference().join("tests/mcbrim_test.3mf")).unwrap();
    // The first plate's eight objects are painted; the others' are not.
    let painted = project.painted(project.plate_index(1));
    assert_eq!(painted.len(), 8);
    assert!(project.painted(project.plate_index(2)).is_empty());
    assert!(project.painted(project.plate_index(3)).is_empty());
    for object in &painted {
        // Every triangle of the object is given, painted or not, and
        // the paint is for the filaments the file says.
        assert_eq!(object.facets.len(), object.solid.triangles.len());
        let mut filaments: Vec<u8> = object.facets.iter().map(|facet| facet.filament).collect();
        filaments.sort_unstable();
        filaments.dedup();
        assert!(
            filaments
                .iter()
                .all(|filament| [0, 2, 3, 4, 7].contains(filament))
        );
        assert!(filaments.iter().any(|filament| *filament > 0));
        // In world coordinates, like the mesh.
        let (low, high) = object.solid.bounds().unwrap();
        assert!(
            object
                .facets
                .iter()
                .flat_map(|facet| &facet.corners)
                .all(|corner| {
                    (0..3).all(|axis| {
                        corner[axis] >= low[axis] - 1e-3 && corner[axis] <= high[axis] + 1e-3
                    })
                })
        );
    }
}
