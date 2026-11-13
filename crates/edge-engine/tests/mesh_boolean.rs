//! "Mesh Boolean" (GLGizmoMeshBoolean.cpp) on a model: two parts of one
//! object made one, in the object's coordinates, the result in the first
//! part's place. Checked by volumes that must add up, by the result being
//! closed and wound outward, by where it stands, and by what is saved and
//! sliced.

use std::path::{Path, PathBuf};

use edge_engine::{
    Bed, BooleanFailure, BooleanOp, Chosen, Model, NewPart, Overrides, PartKind, Saving, Transform,
    Unwatched,
};
use edge_mesh::{Mesh, boolean::check, primitives::cuboid};

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

fn cube(low: [f32; 3], high: [f32; 3]) -> Mesh {
    Mesh::from_triangle_soup(cuboid(low, high))
}

/// An object of a 20 mm cube from an STL (its part named "a.stl"), and a
/// second part, "B": a 20 mm cube placed by `transform`.
fn two_cubes(dir: &Path, transform: Transform) -> (Model, u64, u64, u64) {
    let path: PathBuf = dir.join("a.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&cube([0.0; 3], [20.0; 3]))).unwrap();
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    let b = model
        .add_part(
            Some(&BED),
            object,
            NewPart {
                name: "B".into(),
                kind: PartKind::Normal,
                mesh: cube([0.0; 3], [20.0; 3]),
                transform,
            },
        )
        .unwrap();
    let a = model.objects()[0].shape().part_ids()[0];
    (model, object, a, b)
}

/// A part's mesh in its object's coordinates, as the volume it holds.
fn volume_in_object(model: &Model, at: usize) -> f64 {
    let part = &model.objects()[0].shape().parts()[at];
    let mut mesh = Mesh::default();
    mesh.append(&part.mesh, &part.transform);
    mesh.signed_volume()
}

/// The box round a part in its object's coordinates.
fn box_in_object(model: &Model, at: usize) -> ([f32; 3], [f32; 3]) {
    let part = &model.objects()[0].shape().parts()[at];
    let mut mesh = Mesh::default();
    mesh.append(&part.mesh, &part.transform);
    mesh.bounds().unwrap()
}

/// With `EDGE_KEEP_SAVED` set to a folder, the model is saved there as
/// `name`, to give to the reference (`--info`).
fn keep(model: &Model, name: &str) {
    if let Some(dir) = std::env::var_os("EDGE_KEEP_SAVED") {
        model
            .save(&Path::new(&dir).join(name), &Saving::None, Some(&BED))
            .unwrap();
    }
}

fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-3 * b.abs().max(1.0), "{a} is not {b}");
}

/// B is A moved 10 mm along X: they share half of each.
#[test]
fn each_operation_adds_up_on_two_cubes() {
    let dir = tempfile::tempdir().unwrap();
    // (operation, delete input, volume, parts left, name)
    let cases = [
        (BooleanOp::Union, false, 12000.0, 1, "a.stl - union"),
        (
            BooleanOp::Difference,
            false,
            4000.0,
            2,
            "a.stl - difference",
        ),
        (BooleanOp::Difference, true, 4000.0, 1, "a.stl - difference"),
        (
            BooleanOp::Intersection,
            false,
            4000.0,
            2,
            "a.stl - intersection",
        ),
        (
            BooleanOp::Intersection,
            true,
            4000.0,
            1,
            "a.stl - intersection",
        ),
    ];
    for (op, delete, volume, left, name) in cases {
        let (mut model, object, a, b) =
            two_cubes(dir.path(), Transform::translation([10.0, 0.0, 0.0]));
        let made = model
            .mesh_boolean(Some(&BED), object, a, b, op, delete)
            .unwrap();
        if delete || op == BooleanOp::Union {
            keep(&model, &format!("cubes_{op:?}.3mf").to_lowercase());
        }
        let shape = model.objects()[0].shape().clone();
        assert_eq!(shape.parts().len(), left, "{op:?} {delete}");
        // In the first part's place, under its own new number.
        assert_eq!(shape.part_ids()[0], made);
        assert_ne!(made, a);
        if left == 2 {
            assert_eq!(shape.part_ids()[1], b);
        }
        let part = &shape.parts()[0];
        assert_eq!(part.name.as_deref(), Some(name));
        assert_eq!(part.kind, PartKind::Normal);
        // Closed, wound outward, and as much as the operation leaves.
        assert!(check::is_closed(&part.mesh), "{op:?}");
        assert!(check::defects(&part.mesh).is_clean(), "{op:?}");
        assert!(part.mesh.signed_volume() > 0.0);
        near(volume_in_object(&model, 0), volume);
        // Stored about the middle of its box.
        let (low, high) = part.mesh.bounds().unwrap();
        for axis in 0..3 {
            assert!((low[axis] + high[axis]).abs() < 1e-3);
        }
        // Where the boolean put it, in the object's coordinates.
        let (low, high) = box_in_object(&model, 0);
        let expected = match op {
            BooleanOp::Union => ([0.0, 0.0, 0.0], [30.0, 20.0, 20.0]),
            BooleanOp::Difference => ([0.0, 0.0, 0.0], [10.0, 20.0, 20.0]),
            BooleanOp::Intersection => ([10.0, 0.0, 0.0], [20.0, 20.0, 20.0]),
        };
        for axis in 0..3 {
            assert!(
                (f64::from(low[axis]) - expected.0[axis]).abs() < 1e-4,
                "{op:?}"
            );
            assert!(
                (f64::from(high[axis]) - expected.1[axis]).abs() < 1e-4,
                "{op:?}"
            );
        }
    }
}

/// The parts' own turn, size and mirror are taken into the boolean; a
/// mirrored part stays a solid and the result is wound outward.
#[test]
fn a_mirrored_and_scaled_part_is_taken_in_the_objects_coordinates() {
    let dir = tempfile::tempdir().unwrap();
    // B mirrored in X, half as tall, moved 30 mm along X: it takes
    // [10, 30] x [0, 20] x [0, 10].
    let mirror = Transform {
        rows: [
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 0.5],
            [30.0, 0.0, 0.0],
        ],
    };
    for (op, volume) in [
        (BooleanOp::Union, 8000.0 + 4000.0 - 2000.0),
        (BooleanOp::Difference, 8000.0 - 2000.0),
        (BooleanOp::Intersection, 2000.0),
    ] {
        // The mirrored part first, then second.
        let (mut model, object, a, b) = two_cubes(dir.path(), mirror);
        let made = model
            .mesh_boolean(Some(&BED), object, a, b, op, true)
            .unwrap();
        assert_eq!(model.objects()[0].shape().part_ids(), [made]);
        keep(&model, &format!("mirrored_{op:?}.3mf").to_lowercase());
        near(volume_in_object(&model, 0), volume);
        let (mut model, object, a, b) = two_cubes(dir.path(), mirror);
        model
            .mesh_boolean(Some(&BED), object, b, a, op, false)
            .unwrap();
        let shape = model.objects()[0].shape().clone();
        let suffix = match op {
            BooleanOp::Union => "union",
            BooleanOp::Difference => "difference",
            BooleanOp::Intersection => "intersection",
        };
        // The result is in B's place, the second; A stays first, but
        // for a union, which always deletes the second part it takes.
        let at = if op == BooleanOp::Union { 0 } else { 1 };
        if op != BooleanOp::Union {
            assert_eq!(shape.part_ids()[0], a);
        }
        assert_eq!(shape.parts().len(), at + 1);
        assert_eq!(
            shape.parts()[at].name.as_deref(),
            Some(format!("B - {suffix}").as_str())
        );
        let expected = match op {
            BooleanOp::Union => 10000.0,
            BooleanOp::Difference => 4000.0 - 2000.0,
            BooleanOp::Intersection => 2000.0,
        };
        assert!(shape.parts()[at].mesh.signed_volume() > 0.0);
        assert!(!shape.parts()[at].transform.is_mirrored());
        near(volume_in_object(&model, at), expected);
    }
}

/// Two parts that do not meet: their intersection is empty, said as
/// such, and nothing is changed.
#[test]
fn an_empty_intersection_is_empty_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, a, b) = two_cubes(dir.path(), Transform::translation([40.0, 0.0, 0.0]));
    let before = model.objects()[0].shape().clone();
    let answer = model.mesh_boolean(Some(&BED), object, a, b, BooleanOp::Intersection, true);
    assert!(matches!(answer, Err(BooleanFailure::Empty)), "{answer:?}");
    // A part the other covers whole, taken away, leaves nothing too.
    let (mut covered, object2, a2, b2) =
        two_cubes(dir.path(), Transform::translation([0.0, 0.0, 0.0]));
    let answer = covered.mesh_boolean(Some(&BED), object2, a2, b2, BooleanOp::Difference, false);
    assert!(matches!(answer, Err(BooleanFailure::Empty)), "{answer:?}");
    assert!(std::sync::Arc::ptr_eq(model.objects()[0].shape(), &before));
}

/// A part that does not bound a volume: the boolean fails, names it, and
/// the model is as it was.
#[test]
fn a_failed_boolean_leaves_the_model_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, _, _) = two_cubes(dir.path(), Transform::translation([10.0, 0.0, 0.0]));
    let mut open = cube([0.0; 3], [20.0; 3]);
    open.triangles.truncate(open.triangles.len() - 2);
    let hole = model
        .add_part(
            Some(&BED),
            object,
            NewPart {
                name: "Open".into(),
                kind: PartKind::Normal,
                mesh: open,
                transform: Transform::translation([5.0, 5.0, 5.0]),
            },
        )
        .unwrap();
    let a = model.objects()[0].shape().part_ids()[0];
    let before = model.objects()[0].shape().clone();
    let reach = model.objects()[0].copies()[0].reach();
    let answer = model.mesh_boolean(Some(&BED), object, a, hole, BooleanOp::Union, true);
    match answer {
        Err(BooleanFailure::Failed { part, .. }) => assert_eq!(part.as_deref(), Some("Open")),
        other => panic!("{other:?}"),
    }
    assert!(std::sync::Arc::ptr_eq(model.objects()[0].shape(), &before));
    assert_eq!(model.objects()[0].copies()[0].reach(), reach);
}

/// Only printed parts, two of them, of the object named.
#[test]
fn a_modifier_or_the_same_part_twice_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, a, b) = two_cubes(dir.path(), Transform::translation([10.0, 0.0, 0.0]));
    assert!(matches!(
        model.mesh_boolean(Some(&BED), object, a, a, BooleanOp::Union, true),
        Err(BooleanFailure::Refused(_))
    ));
    model
        .set_part_kind(Some(&BED), object, &[b], PartKind::Modifier)
        .unwrap();
    assert!(matches!(
        model.mesh_boolean(Some(&BED), object, a, b, BooleanOp::Union, true),
        Err(BooleanFailure::Refused(_))
    ));
}

fn sliced(model: &Model) -> String {
    let job = edge_engine::prepare(
        model.place(Some(model.plates()[0].id())).unwrap(),
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    String::from_utf8(sliced.gcode("2001-01-01 at 00:00:00").unwrap().text).unwrap()
}

/// The result saved and opened again is the same part, and slices as it
/// did before it was saved.
#[test]
fn the_result_is_saved_opened_again_and_sliced() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, a, b) =
        two_cubes(dir.path(), Transform::translation([10.0, 0.0, 10.0]));
    model
        .mesh_boolean(Some(&BED), object, a, b, BooleanOp::Difference, false)
        .unwrap();
    let path = dir.path().join("boolean.3mf");
    model.save(&path, &Saving::None, Some(&BED)).unwrap();
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        std::fs::copy(&path, Path::new(&keep).join("boolean.3mf")).unwrap();
    }
    let again = Model::open(&path).unwrap();
    let (was, now) = (model.objects()[0].shape(), again.objects()[0].shape());
    assert_eq!(was.parts().len(), now.parts().len());
    for (x, y) in was.parts().iter().zip(now.parts()) {
        assert_eq!(x.name, y.name);
        assert_eq!(x.mesh.triangles.len(), y.mesh.triangles.len());
        assert!((x.mesh.signed_volume() - y.mesh.signed_volume()).abs() < 1e-3);
    }
    let gcode = sliced(&model);
    assert!(gcode.contains("G1"), "the result sliced to no moves");
    assert_eq!(gcode, sliced(&again));
}
