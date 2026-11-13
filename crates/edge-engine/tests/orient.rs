//! Auto orient on a model (Jobs/OrientJob.cpp): what is oriented for each
//! way it is asked, what stays, and where a turned copy ends up.

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Mesh, Model, OrientScope};
use edge_mesh::primitives::cuboid;

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// The Snapmaker U1's bed, which `mcbrim_test.3mf` is laid out for.
const U1: Bed = Bed {
    min: [0.5, 1.0],
    max: [270.5, 271.0],
};

/// A 30 × 20 × 10 box turned 27° about X and then −41° about Y, as an STL.
fn tilted_box(dir: &Path) -> Model {
    let (sa, ca) = 27f64.to_radians().sin_cos();
    let (sb, cb) = (-41f64).to_radians().sin_cos();
    let turn = |p: [f32; 3]| -> [f32; 3] {
        let [x, y, z] = p.map(f64::from);
        let (y, z) = (ca * y - sa * z, sa * y + ca * z);
        let (x, z) = (cb * x + sb * z, -sb * x + cb * z);
        [x as f32, y as f32, z as f32]
    };
    let mesh = Mesh::from_triangle_soup(
        cuboid([0.0; 3], [30.0, 20.0, 10.0])
            .into_iter()
            .map(|t| t.map(turn)),
    );
    let path = dir.join("tilted.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    Model::open(&path).unwrap()
}

/// The box round a copy as it stands in the scene.
fn reach(model: &Model, copy: u64) -> [[f64; 3]; 2] {
    model.in_scene(Some(&U1), copy).unwrap().1.unwrap()
}

fn only_copy(model: &Model) -> u64 {
    model.objects()[0].copies()[0].id()
}

#[test]
fn a_tilted_box_is_laid_flat_on_the_bed() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = tilted_box(dir.path());
    let copy = only_copy(&model);
    let done = model
        .orient(Some(&U1), None, OrientScope::Selected(&[]))
        .unwrap();
    assert_eq!(done.examined, vec![copy]);
    assert_eq!(done.turned, vec![copy]);
    let [low, high] = reach(&model, copy);
    assert!(low[2].abs() < 1e-9, "on the bed: {low:?}");
    assert!((high[2] - low[2] - 10.0).abs() < 1e-3, "{low:?} {high:?}");
    assert_eq!(model.plate_of(copy), Some(0));
    // Oriented again, it stays.
    let before = model.objects()[0].copies()[0].transform();
    let again = model
        .orient(Some(&U1), None, OrientScope::Selected(&[copy]))
        .unwrap();
    assert!(again.turned.is_empty());
    assert_eq!(model.objects()[0].copies()[0].transform(), before);
}

#[test]
fn a_box_lying_flat_is_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], [30.0, 20.0, 10.0]));
    let path = dir.path().join("flat.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let done = model
        .orient(Some(&U1), None, OrientScope::Selected(&[]))
        .unwrap();
    assert_eq!(done.examined.len(), 1);
    assert!(done.turned.is_empty());
}

#[test]
fn what_stands_on_a_face_already_is_turned_by_a_hair_at_most() {
    // mcbrim_test.3mf: sixteen arms, turned and mirrored, standing on
    // their flat backs on three plates. Their backs are flat to an `f32`,
    // so the direction of most area under them is a millionth off
    // straight down, and that is what wins: the reference's own command
    // line (`--orient 1`) turns them by the same hair, (0.000001,
    // 0.000005, 1) and the like.
    let mut model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let copies: Vec<u64> = model
        .objects()
        .iter()
        .flat_map(|object| object.copies().iter().map(|copy| copy.id()))
        .collect();
    let before: Vec<[[f64; 3]; 2]> = copies.iter().map(|c| reach(&model, *c)).collect();
    let done = model
        .orient(Some(&U1), None, OrientScope::Selected(&[]))
        .unwrap();
    assert_eq!(done.examined.len(), copies.len());
    for (copy, was) in copies.iter().zip(&before) {
        let now = reach(&model, *copy);
        for axis in 0..3 {
            let size = |r: &[[f64; 3]; 2]| r[1][axis] - r[0][axis];
            assert!((size(&now) - size(was)).abs() < 1e-3, "{now:?} {was:?}");
        }
        assert!(now[0][2].abs() < 1e-9);
    }
}

#[test]
fn locked_plates_are_left_out_or_refused_as_the_reference_does() {
    let mut model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let on = |model: &Model, plate: usize| -> Vec<u64> {
        model
            .objects()
            .iter()
            .flat_map(|object| object.copies().iter().map(|copy| copy.id()))
            .filter(|copy| model.plate_of(*copy) == Some(plate))
            .collect()
    };
    let plate2 = on(&model, 1);
    let all: usize = model.objects().iter().map(|o| o.copies().len()).sum();
    model.lock_plate(2, true).unwrap();

    // The plate alone: refused.
    let error = model
        .orient(Some(&U1), None, OrientScope::Plate(2))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "This plate is locked.\nCannot auto-orient on this plate."
    );
    // What is selected all on it: refused.
    let error = model
        .orient(Some(&U1), None, OrientScope::Selected(&plate2[..1]))
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "All the selected objects are on a locked plate.\nCannot auto-orient these objects."
    );
    // Nothing selected: everything but the locked plate.
    let done = model
        .orient(Some(&U1), None, OrientScope::Selected(&[]))
        .unwrap();
    assert_eq!(done.examined.len(), all - plate2.len());
    assert!(done.examined.iter().all(|copy| !plate2.contains(copy)));
    // Another plate: its copies only.
    let plate1 = on(&model, 0);
    let done = model
        .orient(Some(&U1), None, OrientScope::Plate(1))
        .unwrap();
    assert_eq!(done.examined, plate1);
}

#[test]
fn an_empty_plate_is_nothing_to_orient() {
    let mut model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let added = model.add_plate(Some(&U1)).unwrap();
    let done = model
        .orient(Some(&U1), None, OrientScope::Plate(added))
        .unwrap();
    assert!(done.examined.is_empty());
}

#[test]
fn the_same_at_any_thread_count() {
    let dir = tempfile::tempdir().unwrap();
    let model = tilted_box(dir.path());
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut model = model.clone();
                model
                    .orient(Some(&U1), None, OrientScope::Selected(&[]))
                    .unwrap();
                model.objects()[0].copies()[0].transform()
            })
    };
    let one = run(1);
    assert_eq!(one, run(3));
    assert_eq!(one, run(6));
}

#[test]
fn benchy_is_left_standing_as_the_reference_leaves_it() {
    // The reference's command line (`--orient 1`) finds 39 directions for
    // 3DBenchy and keeps it as it stands; measured as it measures, so do we.
    let model = Model::open(&reference("resources/handy_models/3DBenchy.3mf")).unwrap();
    let found = model.objects()[0].orientation_as_command_line().unwrap();
    assert_eq!(found.tried.len(), 39);
    assert_eq!(found.up, [0.0, 0.0, 1.0]);
    // The reference's own line for it: overhang 1431.8, bottom 674.7,
    // bottom of the hull 525.9, contour 103.9, low angle faces 551.3,
    // cost 2.7.
    let costs = found.tried[0].costs;
    let tenths = |v: f32| (v * 10.0).round() / 10.0;
    assert_eq!(
        [
            tenths(costs.overhang),
            tenths(costs.bottom),
            tenths(costs.bottom_hull),
            tenths(costs.contour),
            tenths(costs.area_laf),
            tenths(costs.unprintability)
        ],
        [1431.8, 674.7, 525.9, 103.9, 551.3, 2.7]
    );
}
