//! A model held in memory to be edited: untouched it is its file, and
//! each edit changes what the plates it touches would slice to.

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Loaded, Model, Transform};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// A box of `size` mm with a corner at `at`, as an STL file in `dir`.
fn block(dir: &Path, name: &str, at: [f32; 3], size: [f32; 3]) -> PathBuf {
    let high = [at[0] + size[0], at[1] + size[1], at[2] + size[2]];
    let mesh = Mesh::from_triangle_soup(cuboid(at, high));
    let path = dir.join(name);
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    path
}

/// A bed of 200 by 200 mm from the origin.
const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

fn bits(mesh: &Mesh) -> (Vec<[u32; 3]>, Vec<[u32; 3]>) {
    (
        mesh.vertices
            .iter()
            .map(|vertex| vertex.map(f32::to_bits))
            .collect(),
        mesh.triangles.clone(),
    )
}

/// Everything of what is to be sliced that the slice depends on, to the bit.
fn same(a: &Loaded, b: &Loaded) {
    assert_eq!(a.bodies.len(), b.bodies.len());
    for (one, other) in a.bodies.iter().zip(&b.bodies) {
        assert_eq!(one.filament, other.filament);
        assert_eq!(bits(&one.solid), bits(&other.solid));
        assert_eq!(
            one.negative.as_ref().map(bits),
            other.negative.as_ref().map(bits)
        );
    }
    assert_eq!(a.painted.len(), b.painted.len());
    for (one, other) in a.painted.iter().zip(&b.painted) {
        assert_eq!(bits(&one.solid), bits(&other.solid));
        assert_eq!(
            one.negative.as_ref().map(bits),
            other.negative.as_ref().map(bits)
        );
        assert_eq!(one.facets, other.facets);
    }
    assert_eq!(a.objects.len(), b.objects.len());
    for (one, other) in a.objects.iter().zip(&b.objects) {
        assert_eq!(bits(one), bits(other));
    }
    assert_eq!(a.settings.is_some(), b.settings.is_some());
    assert_eq!(a.unread_paint, b.unread_paint);
    assert_eq!(a.plate, b.plate);
    assert_eq!(a.first_layer_sequence, b.first_layer_sequence);
    assert_eq!(a.bounds, b.bounds);
}

/// Where the middle of the ground a copy takes is in the scene, and the
/// plate it is on by number.
fn standing(model: &Model, copy: u64) -> ([f64; 2], Option<u32>) {
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    (
        [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0],
        model.plate_of(copy).map(|at| model.plates()[at].id()),
    )
}

fn copies(model: &Model) -> Vec<u64> {
    model
        .objects()
        .iter()
        .flat_map(|object| object.copies().iter().map(|copy| copy.id()))
        .collect()
}

#[test]
fn an_untouched_model_is_sliced_as_its_file_is() {
    for (path, plates) in [
        (
            "tests/mcbrim_test.3mf",
            vec![None, Some(1), Some(2), Some(3)],
        ),
        (
            "tests/data/support_corpus/twopart_groups.3mf",
            vec![None, Some(1)],
        ),
        ("resources/handy_models/OrcaToleranceTest.stl", vec![None]),
    ] {
        let path = reference(path);
        let model = Model::open(&path).unwrap();
        for plate in plates {
            let from_file = edge_engine::open(&path, plate).unwrap().place().unwrap();
            let from_model = model.place(plate).unwrap();
            same(&from_file, &from_model);
            assert!(!from_model.placed);
        }
        // The plates are listed as the file lists them.
        let listed = edge_engine::open(&path, None).unwrap().plate_list();
        assert_eq!(model.plate_list(), listed);
    }
    // A plate the file does not have is refused in the file's words.
    let path = reference("tests/mcbrim_test.3mf");
    let model = Model::open(&path).unwrap();
    assert_eq!(
        model.place(Some(9)).err().unwrap().to_string(),
        edge_engine::open(&path, Some(9)).err().unwrap().to_string()
    );
}

#[test]
fn a_copy_that_is_moved_slices_where_it_stands_and_only_its_plate_changes() {
    let path = reference("tests/mcbrim_test.3mf");
    let mut model = Model::open(&path).unwrap();
    let before: Vec<_> = [1, 2, 3].map(|plate| model.stamp(Some(plate))).to_vec();
    let untouched = model.place(Some(1)).unwrap();
    // A copy on plate 2, moved 3 mm along x.
    let copy = model.plates()[1].copies()[0];
    let bed = model
        .settings()
        .and_then(|settings| settings.texts("printable_area"))
        .map(|corners| {
            let corner = |text: &str| -> [f64; 2] {
                let (x, y) = text.split_once('x').unwrap();
                [x.parse().unwrap(), y.parse().unwrap()]
            };
            Bed {
                min: corner(&corners[0]),
                max: corner(&corners[2]),
            }
        })
        .unwrap();
    let (was, reach) = model.in_scene(Some(&bed), copy).unwrap();
    let moved = was.then(&Transform::translation([3.0, 0.0, 0.0]));
    model.set_transforms(Some(&bed), &[(copy, moved)]).unwrap();
    let (now, reached) = model.in_scene(Some(&bed), copy).unwrap();
    assert!((now.rows[3][0] - was.rows[3][0] - 3.0).abs() < 1e-9);
    assert!((reached.unwrap()[0][0] - reach.unwrap()[0][0] - 3.0).abs() < 1e-4);
    assert_eq!(model.plate_of(copy), Some(1));
    // Plates 1 and 3 would slice as before; plate 2 would not.
    assert_eq!(model.stamp(Some(1)), before[0]);
    assert_ne!(model.stamp(Some(2)), before[1]);
    assert_eq!(model.stamp(Some(3)), before[2]);
    same(&untouched, &model.place(Some(1)).unwrap());
    // What is sliced of plate 2 is 3 mm further along x.
    let file = edge_engine::open(&path, Some(2)).unwrap().place().unwrap();
    let edited = model.place(Some(2)).unwrap();
    assert_eq!(file.bodies.len(), edited.bodies.len());
    assert_eq!(file.objects.len(), edited.objects.len());
    let shifts: Vec<f32> = file
        .objects
        .iter()
        .zip(&edited.objects)
        .map(|(was, now)| now.bounds().unwrap().0[0] - was.bounds().unwrap().0[0])
        .filter(|shift| *shift != 0.0)
        .collect();
    assert_eq!(shifts.len(), 1);
    assert!((shifts[0] - 3.0).abs() < 1e-3);
}

#[test]
fn a_bare_model_is_stood_in_the_middle_of_the_bed_once_it_is_edited() {
    let dir = tempfile::tempdir().unwrap();
    let path = block(dir.path(), "block.stl", [5.0, 5.0, 3.0], [20.0, 10.0, 8.0]);
    let mut model = Model::open(&path).unwrap();
    assert!(model.bare());
    assert!(model.plates().is_empty());
    assert_eq!(model.filaments(), [1]);
    let lone = model.place(None).unwrap();
    assert!(!lone.placed);
    same(
        &edge_engine::open(&path, None).unwrap().place().unwrap(),
        &lone,
    );

    // Any edit gives it a place: where slicing would have put it.
    let object = model.objects()[0].id();
    let copy = copies(&model)[0];
    model.rename(Some(&BED), object, " block ").unwrap();
    assert!(!model.bare());
    assert_eq!(model.objects()[0].name(), "block");
    assert_eq!(model.plates().len(), 1);
    assert_eq!(standing(&model, copy), ([100.0, 100.0], Some(1)));
    let placed = model.place(Some(1)).unwrap();
    assert!(placed.placed);
    assert_eq!(placed.bounds, ([90.0, 95.0, 0.0], [110.0, 105.0, 8.0]));
    assert_eq!(model.plate_list()[0].objects, ["block"]);
    assert!(model.rename(Some(&BED), object, "  ").is_err());
}

#[test]
fn a_copy_goes_to_the_plate_it_is_moved_onto_and_off_every_plate_is_not_sliced() {
    let dir = tempfile::tempdir().unwrap();
    let path = block(dir.path(), "block.stl", [0.0; 3], [20.0, 20.0, 10.0]);
    let mut model = Model::open(&path).unwrap();
    let copy = copies(&model)[0];
    assert_eq!(model.add_plate(Some(&BED)).unwrap(), 2);
    assert_eq!(standing(&model, copy), ([100.0, 100.0], Some(1)));
    assert!(model.place(Some(2)).is_err());

    // Onto plate 2, which stands 240 mm along x.
    let onto = |model: &mut Model, x: f64, y: f64, z: f64| {
        model
            .set_transforms(Some(&BED), &[(copy, Transform::translation([x, y, z]))])
            .unwrap();
    };
    onto(&mut model, 300.0, 50.0, 0.0);
    assert_eq!(standing(&model, copy), ([310.0, 60.0], Some(2)));
    let placed = model.place(Some(2)).unwrap();
    assert_eq!(placed.bounds, ([60.0, 50.0, 0.0], [80.0, 70.0, 10.0]));
    assert!(model.place(Some(1)).is_err());
    assert!(!model.overhangs(Some(&BED), copy));

    // Over the edge of plate 2 it is still plate 2's, and in the way.
    onto(&mut model, 230.0, 50.0, 0.0);
    assert_eq!(standing(&model, copy).1, Some(2));
    assert!(model.overhangs(Some(&BED), copy));
    // Between the plates and clear of both it is on neither.
    onto(&mut model, 210.0, 50.0, 0.0);
    assert_eq!(standing(&model, copy), ([220.0, 60.0], None));
    assert!(model.place(Some(1)).is_err());
    assert!(model.place(Some(2)).is_err());
    assert_eq!(model.filaments(), Vec::<usize>::new());
    // Lifted off the bed, it is stood on it again.
    onto(&mut model, 50.0, 50.0, 25.0);
    assert_eq!(standing(&model, copy), ([60.0, 60.0], Some(1)));
    assert_eq!(model.place(Some(1)).unwrap().bounds.0[2], 0.0);
    // A transform that would flatten it is refused, and nothing changes.
    let mut flat = Transform::translation([150.0, 50.0, 0.0]);
    flat.rows[2][2] = 0.0;
    assert!(model.set_transforms(Some(&BED), &[(copy, flat)]).is_err());
    assert!(
        model
            .set_transforms(Some(&BED), &[(copy + 1000, Transform::IDENTITY)])
            .is_err()
    );
    assert_eq!(standing(&model, copy), ([60.0, 60.0], Some(1)));
}

#[test]
fn copies_are_deleted_set_aside_and_given_another_filament() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::new();
    let block = Model::open(&block(dir.path(), "a.stl", [0.0; 3], [10.0, 10.0, 10.0])).unwrap();
    let first = model.import(Some(&BED), &block, None).unwrap()[0];
    let second = model.import(Some(&BED), &block, None).unwrap()[0];
    assert_eq!(model.objects().len(), 2);
    assert_eq!(model.objects()[1].name(), "a.stl");
    assert_eq!(model.plate_list()[0].objects, ["a.stl", "a.stl"]);
    let stamp = model.stamp(Some(1));

    // Not printable: it stays, and is not sliced.
    model.set_printable(Some(&BED), &[second], false).unwrap();
    assert_ne!(model.stamp(Some(1)), stamp);
    assert_eq!(model.plate_list()[0].objects, ["a.stl"]);
    assert_eq!(model.place(Some(1)).unwrap().objects.len(), 1);
    model.set_printable(Some(&BED), &[second], true).unwrap();
    assert_eq!(model.stamp(Some(1)), stamp);

    // Another filament for the second object only.
    let object = model.objects()[1].id();
    model.set_filament(Some(&BED), object, 3).unwrap();
    assert!(model.set_filament(Some(&BED), object, 0).is_err());
    assert_eq!(model.filaments(), [1, 3]);
    assert_eq!(model.objects()[1].shape().filaments(), [3]);
    assert_ne!(
        model.objects()[0].shape().stamp(),
        model.objects()[1].shape().stamp()
    );
    let bodies = model.place(Some(1)).unwrap().bodies;
    assert_eq!(
        bodies.iter().map(|body| body.filament).collect::<Vec<_>>(),
        [1, 3]
    );
    // A name changes nothing that is sliced.
    let stamp = model.stamp(Some(1));
    model.rename(Some(&BED), object, "other").unwrap();
    assert_eq!(model.stamp(Some(1)), stamp);

    // Deleting the last copy of an object deletes the object.
    model.remove(Some(&BED), &[first]).unwrap();
    assert_eq!(model.objects().len(), 1);
    assert_eq!(model.objects()[0].name(), "other");
    assert_eq!(model.plates()[0].copies(), [second]);
    assert!(model.remove(Some(&BED), &[first]).is_err());
}

#[test]
fn plates_are_added_and_taken_away_and_what_stood_on_one_is_kept_aside() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::new();
    let block = Model::open(&block(dir.path(), "a.stl", [0.0; 3], [10.0, 10.0, 10.0])).unwrap();
    let first = model.import(Some(&BED), &block, None).unwrap()[0];
    assert_eq!(model.add_plate(Some(&BED)).unwrap(), 2);
    assert_eq!(model.add_plate(Some(&BED)).unwrap(), 3);
    let second = model.import(Some(&BED), &block, Some(2)).unwrap()[0];
    let third = model.import(Some(&BED), &block, Some(3)).unwrap()[0];
    // Three plates stand two to a row.
    assert_eq!(standing(&model, second), ([340.0, 100.0], Some(2)));
    assert_eq!(standing(&model, third), ([100.0, -140.0], Some(3)));
    assert!(model.remove_plate(Some(&BED), 7).is_err());

    // Plate 2 goes: plate 3 is plate 2 now and stands where plate 2 stood,
    // with what is on it; what stood on the old plate 2 is beside them.
    model.remove_plate(Some(&BED), 2).unwrap();
    let ids: Vec<u32> = model.plates().iter().map(|plate| plate.id()).collect();
    assert_eq!(ids, [1, 2]);
    assert_eq!(standing(&model, first), ([100.0, 100.0], Some(1)));
    assert_eq!(standing(&model, third), ([340.0, 100.0], Some(2)));
    assert_eq!(standing(&model, second), ([100.0, -140.0], None));
    assert_eq!(model.objects().len(), 3);
    // A plate added again comes to stand under it and takes it.
    assert_eq!(model.add_plate(Some(&BED)).unwrap(), 3);
    assert_eq!(standing(&model, second), ([100.0, -140.0], Some(3)));
    assert_eq!(
        model.place(Some(3)).unwrap().bounds,
        ([95.0, 95.0, 0.0], [105.0, 105.0, 10.0])
    );
    // The last plate cannot go.
    model.remove_plate(Some(&BED), 3).unwrap();
    model.remove_plate(Some(&BED), 2).unwrap();
    assert!(model.remove_plate(Some(&BED), 1).is_err());
    assert_eq!(model.plates().len(), 1);
}

/// Whether no two copies on a plate are nearer than `apart`, and all on the bed.
fn apart(model: &Model, plate: usize, apart: f64) -> bool {
    let boxes: Vec<[[f64; 3]; 2]> = model.plates()[plate]
        .copies()
        .iter()
        .map(|copy| {
            let object = model
                .objects()
                .iter()
                .find(|object| object.copies().iter().any(|held| held.id() == *copy))
                .unwrap();
            let held = object.copies().iter().find(|held| held.id() == *copy);
            held.unwrap().reach().unwrap()
        })
        .collect();
    let on_bed = boxes.iter().all(|[low, high]| {
        low[0] >= BED.min[0] - 1e-9
            && low[1] >= BED.min[1] - 1e-9
            && high[0] <= BED.max[0] + 1e-9
            && high[1] <= BED.max[1] + 1e-9
            && low[2].abs() < 1e-6
    });
    let clear = boxes.iter().enumerate().all(|(at, a)| {
        boxes[at + 1..].iter().all(|b| {
            a[1][0] + apart <= b[0][0] + 1e-9
                || b[1][0] + apart <= a[0][0] + 1e-9
                || a[1][1] + apart <= b[0][1] + 1e-9
                || b[1][1] + apart <= a[0][1] + 1e-9
        })
    });
    on_bed && clear
}

#[test]
fn what_is_added_and_cloned_finds_a_free_place_and_arranging_keeps_things_apart() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::new();
    let wide = Model::open(&block(
        dir.path(),
        "wide.stl",
        [3.0, 4.0, 5.0],
        [60.0, 30.0, 10.0],
    ))
    .unwrap();
    let small = Model::open(&block(
        dir.path(),
        "small.stl",
        [0.0; 3],
        [20.0, 20.0, 20.0],
    ))
    .unwrap();
    // The first in the middle of the bed, standing on it.
    let first = model.import(Some(&BED), &wide, None).unwrap()[0];
    assert_eq!(standing(&model, first), ([100.0, 100.0], Some(1)));
    let mut all = vec![first];
    for _ in 0..3 {
        all.extend(model.import(Some(&BED), &small, None).unwrap());
    }
    // A clone is its own object, of the same shape and name.
    all.extend(model.clone_copies(Some(&BED), &[first], None).unwrap());
    assert_eq!(model.objects().len(), 5);
    assert_eq!(model.objects()[4].name(), "wide.stl");
    assert_eq!(
        model.objects()[4].shape().stamp(),
        model.objects()[0].shape().stamp()
    );
    assert!(apart(&model, 0, 6.0));

    // Thrown together, then arranged.
    let heap: Vec<(u64, Transform)> = all
        .iter()
        .map(|copy| (*copy, Transform::translation([90.0, 90.0, 0.0])))
        .collect();
    model.set_transforms(Some(&BED), &heap).unwrap();
    assert!(!apart(&model, 0, 6.0));
    assert_eq!(model.arrange(Some(&BED), None).unwrap(), 0);
    assert!(apart(&model, 0, 6.0));
    assert_eq!(model.plates()[0].copies().len(), 5);
    // The same again changes nothing: an arrangement is the same every time.
    let stamp = model.stamp(Some(1));
    model.arrange(Some(&BED), Some(1)).unwrap();
    assert_eq!(model.stamp(Some(1)), stamp);

    // More than the bed holds: the rest is put beside the plate.
    let mut full = Model::new();
    let big = Model::open(&block(dir.path(), "big.stl", [0.0; 3], [120.0, 120.0, 5.0])).unwrap();
    let one = full.import(Some(&BED), &big, None).unwrap()[0];
    let two = full.import(Some(&BED), &big, None).unwrap()[0];
    assert_eq!(standing(&full, one).1, Some(1));
    assert_eq!(standing(&full, two), ([340.0, 100.0], None));
    assert_eq!(full.plate_list()[0].objects.len(), 1);
    assert!(full.arrange(None, None).is_err());
}
