//! The tools that change an object's shape or how it lies: laying it on a
//! face, mirroring it, splitting it.

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Model, Reach};
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

fn first_copy(model: &Model) -> u64 {
    model.objects()[0].copies()[0].id()
}

fn reach(model: &Model, copy: u64) -> Reach {
    model.in_scene(Some(&BED), copy).unwrap().1.unwrap()
}

fn size(reach: &Reach) -> [f64; 3] {
    std::array::from_fn(|axis| ((reach[1][axis] - reach[0][axis]) * 1e6).round() / 1e6)
}

fn middle(reach: &Reach) -> [f64; 2] {
    std::array::from_fn(|axis| ((reach[0][axis] + reach[1][axis]) / 2.0 * 1e6).round() / 1e6)
}

#[test]
fn a_box_offers_its_six_sides_and_is_laid_on_the_one_chosen() {
    let dir = tempfile::tempdir().unwrap();
    let path = boxes(
        dir.path(),
        "box.stl",
        &[([5.0, 5.0, 0.0], [30.0, 20.0, 10.0])],
    );
    let mut model = Model::open(&path).unwrap();
    let copy = first_copy(&model);
    let faces = model.faces(copy).unwrap();
    assert_eq!(faces.len(), 6);
    let areas: Vec<f64> = faces.iter().map(|face| face.area.round()).collect();
    assert_eq!(areas, [600.0, 600.0, 300.0, 300.0, 200.0, 200.0]);
    // Each face's normal is a side's.
    for face in &faces {
        assert_eq!(face.normal.iter().filter(|v| v.abs() == 1.0).count(), 1);
    }

    // Laid on its 30 by 10 side, it stands 20 mm tall, on the bed, where
    // it stood across the bed.
    // An edit gives the bare model its place in the middle of the bed.
    let object = model.objects()[0].id();
    model.rename(Some(&BED), object, "box").unwrap();
    let before = reach(&model, copy);
    model
        .lay_on_face(Some(&BED), copy, [0.0, -1.0, 0.0])
        .unwrap();
    let after = reach(&model, copy);
    assert_eq!(size(&after), [30.0, 10.0, 20.0]);
    assert!(after[0][2].abs() < 1e-9);
    assert_eq!(middle(&after), middle(&before));
    // Its sides as it now lies: the largest are now upright.
    let faces = model.faces(copy).unwrap();
    assert_eq!(faces.len(), 6);
    let placed = model.place(Some(1)).unwrap();
    assert!((placed.bounds.1[2] - 20.0).abs() < 1e-4);

    // Laid on the face that was its top: upside down, as tall as before.
    model
        .lay_on_face(Some(&BED), copy, [0.0, 0.0, 1.0])
        .unwrap();
    assert_eq!(size(&reach(&model, copy)), [30.0, 20.0, 10.0]);
    assert!(
        model
            .lay_on_face(Some(&BED), copy, [0.0, 0.0, 0.0])
            .is_err()
    );
    assert!(model.lay_on_face(Some(&BED), 0, [0.0, 0.0, 1.0]).is_err());
}

#[test]
fn faces_follow_the_size_a_copy_is_printed_at() {
    // A wedge: a box with one top edge cut off, so one face is slanted.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wedge.stl");
    let p = |x: f32, y: f32, z: f32| [x, y, z];
    let (a, b, c, d) = (
        p(0.0, 0.0, 0.0),
        p(20.0, 0.0, 0.0),
        p(20.0, 10.0, 0.0),
        p(0.0, 10.0, 0.0),
    );
    let (e, f) = (p(0.0, 0.0, 10.0), p(0.0, 10.0, 10.0));
    let soup = vec![
        [a, c, b],
        [a, d, c],
        [a, b, e],
        [d, f, c],
        [a, e, f],
        [a, f, d],
        [b, c, f],
        [b, f, e],
    ];
    let mesh = Mesh::from_triangle_soup(soup);
    assert!(mesh.signed_volume() > 0.0);
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let copy = first_copy(&model);
    let faces = model.faces(copy).unwrap();
    assert_eq!(faces.len(), 5);
    // The slanted face is the largest: 10 by the hypotenuse.
    let slant = 10.0 * (20f64.powi(2) + 10f64.powi(2)).sqrt();
    assert!((faces[0].area - slant).abs() < 1e-6, "{}", faces[0].area);
    let n = faces[0].normal;
    model.lay_on_face(Some(&BED), copy, n).unwrap();
    let after = reach(&model, copy);
    assert!(after[0][2].abs() < 1e-9);
    // On its slanted face the wedge's highest point is the corner opposite,
    // 20 * 10 / hypotenuse above it.
    let height = 200.0 / (20f64.powi(2) + 10f64.powi(2)).sqrt();
    assert!((after[1][2] - height).abs() < 1e-6, "{after:?}");
}

#[test]
fn a_mirror_is_kept_in_the_copy_and_what_is_sliced_stays_a_solid() {
    let dir = tempfile::tempdir().unwrap();
    let path = boxes(
        dir.path(),
        "ell.stl",
        &[
            ([0.0; 3], [6.0, 10.0, 16.0]),
            ([6.0, 0.0, 0.0], [24.0, 10.0, 4.0]),
        ],
    );
    let mut model = Model::open(&path).unwrap();
    let copy = first_copy(&model);
    let object = model.objects()[0].id();
    model.rename(Some(&BED), object, "ell").unwrap();
    let before = reach(&model, copy);
    let solid = model.place(Some(1)).unwrap().bodies[0]
        .solid
        .signed_volume();

    for axis in [0, 1, 2] {
        model.mirror(Some(&BED), &[copy], axis).unwrap();
        let transform = model.objects()[0].copies()[0].transform();
        assert!(transform.is_mirrored());
        // The mesh is the file's; the copy turns it inside out, and what is
        // sliced is wound back to a solid of the same volume.
        let placed = model.place(Some(1)).unwrap();
        let volume = placed.bodies[0].solid.signed_volume();
        assert!((volume - solid).abs() < 1e-3, "{volume} {solid}");
        // Where it stood, on the bed.
        assert_eq!(size(&reach(&model, copy)), size(&before));
        assert_eq!(middle(&reach(&model, copy)), middle(&before));
        assert!(reach(&model, copy)[0][2].abs() < 1e-9);
        model.mirror(Some(&BED), &[copy], axis).unwrap();
        assert!(!model.objects()[0].copies()[0].transform().is_mirrored());
    }
    assert!(model.mirror(Some(&BED), &[copy], 3).is_err());
    assert!(model.mirror(Some(&BED), &[], 0).is_err());
}
