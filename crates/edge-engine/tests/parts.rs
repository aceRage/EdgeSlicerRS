//! The parts of an object as things to change: a filament of their own,
//! a name, a place within the object, a kind; deleted, split, mirrored,
//! added. Each is checked by what is then sliced.

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Model, NewPart, PartKind, Reach, Transform};
use edge_mesh::{Mesh, primitives::cuboid};

/// Boxes, each `(corner, size)`, as one mesh in an STL file in `dir`.
fn boxes(dir: &Path, name: &str, boxes: &[([f32; 3], [f32; 3])]) -> PathBuf {
    let soup = boxes.iter().flat_map(|(at, size)| {
        let high = [at[0] + size[0], at[1] + size[1], at[2] + size[2]];
        cuboid(*at, high)
    });
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join(name);
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    path
}

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

/// Two boxes 10 mm apart in one STL, split to two parts: "two.stl_1" (10
/// mm wide, 10 tall) and "two.stl_2" (10 wide, 20 tall).
fn two_parts(dir: &Path) -> Model {
    let path = boxes(
        dir,
        "two.stl",
        &[
            ([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            ([20.0, 0.0, 0.0], [10.0, 10.0, 20.0]),
        ],
    );
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(Some(&BED), object).unwrap();
    assert_eq!(model.objects()[0].parts(), 2);
    model
}

fn ids(model: &Model) -> (u64, u64, Vec<u64>) {
    let object = &model.objects()[0];
    (
        object.id(),
        object.copies()[0].id(),
        object.shape().part_ids().to_vec(),
    )
}

fn reach(model: &Model, copy: u64) -> Reach {
    model.in_scene(Some(&BED), copy).unwrap().1.unwrap()
}

fn round(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

#[test]
fn a_part_prints_in_its_own_filament_and_back_in_its_objects() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (object, _, parts) = ids(&model);
    let before = model.place(Some(1)).unwrap();
    assert_eq!(before.bodies.len(), 1);

    model
        .set_part_filament(Some(&BED), object, &[parts[1]], Some(2))
        .unwrap();
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts()[1].filament, 2);
    assert_eq!(shape.parts()[1].own_filament, Some(2));
    // The part keeps its number; the shape is another.
    assert_eq!(shape.part_ids(), parts.as_slice());
    assert_eq!(model.filaments(), [1, 2]);
    let placed = model.place(Some(1)).unwrap();
    let filaments: Vec<usize> = placed.bodies.iter().map(|body| body.filament).collect();
    assert_eq!(filaments, [1, 2]);
    // The taller box is the one in filament 2.
    let tall = placed
        .bodies
        .iter()
        .find(|body| body.filament == 2)
        .unwrap();
    let (low, high) = tall.solid.bounds().unwrap();
    assert!((high[2] - low[2] - 20.0).abs() < 1e-4);

    // "default": the object's filament, 1 here.
    model
        .set_part_filament(Some(&BED), object, &[parts[1]], None)
        .unwrap();
    assert_eq!(model.filaments(), [1]);
    assert_eq!(model.objects()[0].shape().parts()[1].own_filament, None);

    // The object's filament set: every part follows it.
    model
        .set_part_filament(Some(&BED), object, &[parts[0]], Some(3))
        .unwrap();
    model.set_filament(Some(&BED), object, 2).unwrap();
    let shape = model.objects()[0].shape().clone();
    assert!(
        shape
            .parts()
            .iter()
            .all(|part| part.filament == 2 && part.own_filament.is_none())
    );
    assert_eq!(model.objects()[0].filament(), Some(2));
    // A part set to "default" now follows filament 2.
    model
        .set_part_filament(Some(&BED), object, &[parts[0]], None)
        .unwrap();
    assert_eq!(model.objects()[0].shape().parts()[0].filament, 2);

    assert!(
        model
            .set_part_filament(Some(&BED), object, &[parts[0]], Some(0))
            .is_err()
    );
    assert!(
        model
            .set_part_filament(Some(&BED), object, &[999], Some(1))
            .is_err()
    );
}

#[test]
fn a_part_moved_within_its_object_is_sliced_where_it_now_is() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (object, copy, parts) = ids(&model);
    let before = reach(&model, copy);
    assert_eq!(round(before[1][0] - before[0][0]), 30.0);

    // The short box 15 mm further out along -X, within the object.
    let moved = Transform::translation([-15.0, 0.0, 0.0]);
    model
        .set_part_transforms(Some(&BED), object, &[(parts[0], moved)])
        .unwrap();
    let after = reach(&model, copy);
    assert_eq!(round(after[1][0] - after[0][0]), 45.0);
    assert_eq!(round(after[0][0]), round(before[0][0] - 15.0));
    let placed = model.place(Some(1)).unwrap();
    assert!((f64::from(placed.bounds.1[0] - placed.bounds.0[0]) - 45.0).abs() < 1e-3);
    assert_eq!(model.objects()[0].shape().parts()[0].transform, moved);

    // Lifted 5 mm: the object still stands on its other part.
    let lifted = Transform::translation([-15.0, 0.0, 5.0]);
    model
        .set_part_transforms(Some(&BED), object, &[(parts[0], lifted)])
        .unwrap();
    let after = reach(&model, copy);
    assert!(after[0][2].abs() < 1e-9);
    assert_eq!(round(after[1][2]), 20.0);

    // Both parts lifted: the object is brought down to the bed again, as
    // the reference brings down a flying instance.
    let up = Transform::translation([0.0, 0.0, 7.0]);
    model
        .set_part_transforms(Some(&BED), object, &[(parts[0], up), (parts[1], up)])
        .unwrap();
    let after = reach(&model, copy);
    assert!(after[0][2].abs() < 1e-9);
    let placed = model.place(Some(1)).unwrap();
    assert!(placed.bounds.0[2].abs() < 1e-4);

    let flat = Transform::scaling(0.0);
    assert!(
        model
            .set_part_transforms(Some(&BED), object, &[(parts[0], flat)])
            .is_err()
    );
}

#[test]
fn the_last_solid_part_stays() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (object, copy, parts) = ids(&model);
    // Neither both parts nor the last one can go, nor be made another kind.
    let error = model
        .remove_parts(Some(&BED), object, &parts)
        .unwrap_err()
        .to_string();
    assert_eq!(error, "Deleting the last solid part is not allowed.");
    let error = model
        .set_part_kind(Some(&BED), object, &parts, PartKind::Modifier)
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "The type of the last solid object part is not to be changed."
    );

    // The tall part given a filament of its own, then the short one
    // deleted: its filament becomes the object's.
    model
        .set_part_filament(Some(&BED), object, &[parts[1]], Some(2))
        .unwrap();
    model.remove_parts(Some(&BED), object, &[parts[0]]).unwrap();
    let held = &model.objects()[0];
    assert_eq!(held.parts(), 1);
    assert_eq!(held.filament(), Some(2));
    assert_eq!(held.shape().parts()[0].own_filament, None);
    assert_eq!(held.shape().parts()[0].filament, 2);
    // What is left stands where it stood.
    let left = reach(&model, copy);
    assert_eq!(round(left[1][0] - left[0][0]), 10.0);
    assert_eq!(round(left[1][2]), 20.0);
    let error = model
        .remove_parts(Some(&BED), object, &[parts[1]])
        .unwrap_err()
        .to_string();
    assert_eq!(error, "Deleting the last solid part is not allowed.");
}

#[test]
fn a_part_made_a_modifier_is_kept_and_not_printed() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (object, copy, parts) = ids(&model);
    model
        .set_part_kind(Some(&BED), object, &[parts[1]], PartKind::Modifier)
        .unwrap();
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts()[1].kind, PartKind::Modifier);
    assert_eq!(shape.part_ids(), parts.as_slice());
    // Only the short box is printed, and the modifier is followed: kept
    // with its object for what it sets.
    let placed = model.place(Some(1)).unwrap();
    assert!((placed.bounds.1[2] - 10.0).abs() < 1e-4);
    assert_eq!(placed.own.len(), 1);
    assert_eq!(placed.own[0].volumes[1].kind, PartKind::Modifier);
    assert_eq!(round(reach(&model, copy)[1][2]), 10.0);
    // A negative part takes away from it: the reader's own bodies.
    model
        .set_part_kind(Some(&BED), object, &[parts[1]], PartKind::Negative)
        .unwrap();
    let placed = model.place(Some(1)).unwrap();
    assert!(placed.bodies.iter().any(|body| body.negative.is_some()));
    // And back.
    model
        .set_part_kind(Some(&BED), object, &[parts[1]], PartKind::Normal)
        .unwrap();
    assert!((model.place(Some(1)).unwrap().bounds.1[2] - 20.0).abs() < 1e-4);
    assert!(
        model
            .set_part_kind(Some(&BED), object, &[parts[1]], PartKind::Normal)
            .is_err()
    );
}

#[test]
fn a_part_of_several_pieces_splits_in_its_place() {
    let dir = tempfile::tempdir().unwrap();
    // Three shells: split to parts, then the first two joined again by
    // making a part of a mesh of two shells.
    let path = boxes(
        dir.path(),
        "three.stl",
        &[
            ([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            ([20.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
            ([40.0, 0.0, 0.0], [10.0, 10.0, 5.0]),
        ],
    );
    let mut model = Model::open(&path).unwrap();
    let (object, copy, _) = ids(&model);
    let two = Mesh::from_triangle_soup(
        cuboid([60.0, 0.0, 0.0], [70.0, 10.0, 10.0])
            .into_iter()
            .chain(cuboid([80.0, 0.0, 0.0], [90.0, 10.0, 10.0])),
    );
    // An object of one part of three pieces and an added part of two.
    let added = model
        .add_part(
            Some(&BED),
            object,
            NewPart {
                name: "pair".into(),
                kind: PartKind::Normal,
                mesh: two,
                transform: Transform::IDENTITY,
            },
        )
        .unwrap();
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts().len(), 2);
    assert_eq!(shape.parts()[1].name.as_deref(), Some("pair"));
    assert!(shape.part_splits(0) && shape.part_splits(1));
    // "Split to parts" of the object is for an object of one part.
    assert!(!shape.splits_to_parts());
    let before = reach(&model, copy);
    model.split_part(Some(&BED), object, added).unwrap();
    let shape = model.objects()[0].shape().clone();
    let names: Vec<_> = shape
        .parts()
        .iter()
        .map(|part| part.name.clone().unwrap_or_default())
        .collect();
    assert_eq!(names, ["", "pair_1", "pair_2"]);
    assert!(
        shape.parts()[1..]
            .iter()
            .all(|part| part.own_filament == Some(1))
    );
    assert_eq!(reach(&model, copy), before);
    // A part of one piece does not split.
    let error = model
        .split_part(Some(&BED), object, shape.part_ids()[1])
        .unwrap_err()
        .to_string();
    assert_eq!(
        error,
        "The target object contains only one part and can not be split."
    );
}

#[test]
fn a_part_is_mirrored_about_its_own_middle() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (_, copy, parts) = ids(&model);
    let before = reach(&model, copy);
    model
        .mirror_parts(Some(&BED), copy, &[parts[1]], 0)
        .unwrap();
    let shape = model.objects()[0].shape().clone();
    assert!(shape.parts()[1].transform.is_mirrored());
    assert!(!shape.parts()[0].transform.is_mirrored());
    // A box mirrored about its middle fills the same room.
    let after = reach(&model, copy);
    for axis in 0..3 {
        assert_eq!(round(after[0][axis]), round(before[0][axis]));
        assert_eq!(round(after[1][axis]), round(before[1][axis]));
    }
    // Sliced, it is still a solid of the same volume.
    let placed = model.place(Some(1)).unwrap();
    let volume: f64 = placed
        .bodies
        .iter()
        .map(|body| body.solid.signed_volume())
        .sum();
    assert!((volume - 3000.0).abs() < 1e-3, "{volume}");
}

#[test]
fn a_part_is_renamed_and_a_cube_added_beside_the_object() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = two_parts(dir.path());
    let (object, copy, parts) = ids(&model);
    model
        .rename_part(Some(&BED), object, parts[0], "left")
        .unwrap();
    assert_eq!(
        model.objects()[0].shape().parts()[0].name.as_deref(),
        Some("left")
    );
    assert!(
        model
            .rename_part(Some(&BED), object, parts[0], "  ")
            .is_err()
    );

    let before = reach(&model, copy);
    let side = 20.0;
    let transform = model.beside(Some(&BED), copy, [side; 3]).unwrap();
    let half = (side / 2.0) as f32;
    let cube = Mesh::from_triangle_soup(cuboid([-half; 3], [half; 3]));
    model
        .add_part(
            Some(&BED),
            object,
            NewPart {
                name: "Generic-Cube".into(),
                kind: PartKind::Modifier,
                mesh: cube.clone(),
                transform,
            },
        )
        .unwrap();
    // A modifier: nothing printed moves.
    assert_eq!(reach(&model, copy), before);
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts()[2].kind, PartKind::Modifier);
    assert_eq!(shape.parts()[2].own_filament, None);
    // As a part: right of the object's box and on the bed.
    model
        .add_part(
            Some(&BED),
            object,
            NewPart {
                name: "Generic-Cube".into(),
                kind: PartKind::Normal,
                mesh: cube,
                transform,
            },
        )
        .unwrap();
    let after = reach(&model, copy);
    assert_eq!(round(after[1][0]), round(before[1][0] + side));
    assert_eq!(round(after[0][1]), round(before[0][1]));
    assert!(after[0][2].abs() < 1e-9);
    assert_eq!(round(after[1][2]), 20.0);
}
