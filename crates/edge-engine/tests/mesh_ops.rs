//! A part given another mesh ("Simplify"'s "Apply", "Repair"): the part
//! keeps its place and settings, the copies stand on the bed again, paint
//! goes, what was repaired is saved with it and read back, and the result
//! slices. And the object list's warning, worked out as the reference's
//! statistics are.

use std::path::Path;

use edge_engine::{
    Bed, Chosen, MeshStat, Model, Overrides, PaintKind, Saving, Unwatched, open_edges, simplify,
};
use edge_mesh::Mesh;

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

/// A sphere of radius 10 about (0, 0, 10): 64 by 32 bands.
fn sphere() -> Mesh {
    let (rings, sectors) = (32usize, 64usize);
    let point = |ring: usize, sector: usize| {
        if ring == 0 || ring == rings {
            return [0.0, 0.0, if ring == 0 { 20.0 } else { 0.0 }];
        }
        let theta = std::f64::consts::PI * ring as f64 / rings as f64;
        let phi = 2.0 * std::f64::consts::PI * (sector % sectors) as f64 / sectors as f64;
        [
            (10.0 * theta.sin() * phi.cos()) as f32,
            (10.0 * theta.sin() * phi.sin()) as f32,
            (10.0 + 10.0 * theta.cos()) as f32,
        ]
    };
    let mut soup = Vec::new();
    for ring in 0..rings {
        for sector in 0..sectors {
            let (a, b) = (point(ring, sector), point(ring, sector + 1));
            let (c, d) = (point(ring + 1, sector + 1), point(ring + 1, sector));
            if ring > 0 {
                soup.push([a, d, b]);
            }
            if ring + 1 < rings {
                soup.push([b, d, c]);
            }
        }
    }
    Mesh::from_triangle_soup(soup)
}

fn opened(dir: &Path, mesh: &Mesh) -> (Model, u64, u64) {
    let path = dir.join("ball.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(mesh)).unwrap();
    let model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    let part = model.objects()[0].shape().part_ids()[0];
    (model, object, part)
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

/// The lowest point of the object's first copy on the bed.
fn lowest(model: &Model) -> f64 {
    let object = &model.objects()[0];
    let part = &object.shape().parts()[0];
    let copy = &object.copies()[0];
    part.mesh
        .vertices
        .iter()
        .map(|v| {
            part.transform
                .then(&copy.transform())
                .apply(v.map(f64::from))[2]
        })
        .fold(f64::MAX, f64::min)
}

#[test]
fn a_simplified_part_is_applied_saved_opened_again_and_sliced() {
    let dir = tempfile::tempdir().unwrap();
    let ball = sphere();
    assert_eq!(open_edges(&ball), 0);
    let (mut model, object, part) = opened(dir.path(), &ball);
    let (found, mesh) = model.mesh_of_part(object, None).unwrap();
    assert_eq!(found, part);
    let mut fewer = (*mesh).clone();
    let stats = simplify::quadric_edge_collapse(
        &mut fewer,
        mesh.triangles.len() as u32 / 2,
        None,
        &mut |_| {},
        &|| false,
    )
    .unwrap();
    assert!(stats.triangles_after <= mesh.triangles.len() / 2);
    let painted = model
        .set_part_mesh(Some(&BED), object, part, fewer.clone(), MeshStat::default())
        .unwrap();
    assert!(!painted);
    let now = &model.objects()[0].shape().parts()[0];
    assert_eq!(now.mesh.triangles.len(), stats.triangles_after);
    assert_eq!(model.objects()[0].shape().part_ids(), [part]);
    assert!(
        lowest(&model).abs() < 1e-4,
        "stands on the bed: {}",
        lowest(&model)
    );
    let errors = model.mesh_errors(object, None).unwrap();
    assert!(!errors.shown(), "{errors:?}");

    let path = dir.path().join("simplified.3mf");
    model.save(&path, &Saving::None, Some(&BED)).unwrap();
    let again = Model::open(&path).unwrap();
    let read = &again.objects()[0].shape().parts()[0].mesh;
    assert_eq!(read.triangles.len(), fewer.triangles.len());
    assert!((read.signed_volume() - fewer.signed_volume()).abs() < 1e-3);
    let gcode = sliced(&model);
    assert!(gcode.contains("G1"));
    assert_eq!(gcode, sliced(&again));
}

#[test]
fn what_was_repaired_is_kept_with_the_part_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, part) = opened(dir.path(), &sphere());
    let mesh = (*model.mesh_of_part(object, None).unwrap().1).clone();
    let stat = MeshStat {
        edges_fixed: 3,
        degenerate_facets: 1,
        facets_removed: 1,
        facets_reversed: 2,
        backwards_edges: 0,
    };
    model
        .set_part_mesh(Some(&BED), object, part, mesh, stat)
        .unwrap();
    let errors = model.mesh_errors(object, Some(part)).unwrap();
    assert_eq!(errors.repaired, stat);
    assert_eq!(errors.repaired.count(), 7);
    assert!(errors.shown() && errors.manifold());
    let path = dir.path().join("repaired.3mf");
    model.save(&path, &Saving::None, Some(&BED)).unwrap();
    let again = Model::open(&path).unwrap();
    let object = again.objects()[0].id();
    assert_eq!(again.mesh_errors(object, None).unwrap().repaired, stat);
}

#[test]
fn open_edges_are_counted_as_the_reference_counts_them() {
    // A sphere with two neighbouring triangles taken out of it: a hole
    // whose rim has four edges.
    let mut ball = sphere();
    let first = ball.triangles.len() / 2;
    let a = ball.triangles[first];
    let taken: Vec<usize> = (0..ball.triangles.len())
        .filter(|&i| {
            let t = ball.triangles[i];
            i == first || (t.contains(&a[0]) && t.contains(&a[1]))
        })
        .collect();
    assert_eq!(taken.len(), 2);
    for i in taken.into_iter().rev() {
        ball.triangles.remove(i);
    }
    assert_eq!(open_edges(&ball), 4);
    let dir = tempfile::tempdir().unwrap();
    let (model, object, _) = opened(dir.path(), &ball);
    let errors = model.mesh_errors(object, None).unwrap();
    assert_eq!(errors.open_edges, 4);
    assert!(errors.shown() && !errors.manifold());
}

#[test]
fn paint_on_the_object_goes_with_a_new_mesh() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, part) = opened(dir.path(), &sphere());
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Supports,
            &[(part, vec![(0, "4".to_string()), (5, "8".to_string())])],
        )
        .unwrap();
    assert!(model.objects()[0].shape().parts()[0].has_paint(PaintKind::Supports));
    let mesh = (*model.mesh_of_part(object, None).unwrap().1).clone();
    let painted = model
        .set_part_mesh(Some(&BED), object, part, mesh, MeshStat::default())
        .unwrap();
    assert!(painted);
    assert!(!model.objects()[0].shape().parts()[0].has_paint(PaintKind::Supports));
}

#[test]
fn an_object_of_several_parts_needs_a_part_named() {
    let dir = tempfile::tempdir().unwrap();
    let mut two = sphere();
    two.append(
        &sphere(),
        &edge_mesh::Transform::translation([30.0, 0.0, 0.0]),
    );
    let path = dir.path().join("two.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&two)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(Some(&BED), object).unwrap();
    assert_eq!(model.objects()[0].shape().parts().len(), 2);
    assert!(model.mesh_of_part(object, None).is_err());
    let second = model.objects()[0].shape().part_ids()[1];
    assert_eq!(model.mesh_of_part(object, Some(second)).unwrap().0, second);
}

#[test]
fn the_bed_is_found_in_a_parts_own_coordinates_and_a_repair_is_put_in() {
    let dir = tempfile::tempdir().unwrap();
    let (mut model, object, part) = opened(dir.path(), &sphere());
    let down = model.part_bed_direction(object, part).unwrap();
    assert!((down[2] + 1.0).abs() < 1e-12 && down[0].abs() < 1e-12 && down[1].abs() < 1e-12);
    let printed = model.printed_meshes(object, &[]).unwrap();
    assert_eq!(printed.len(), 1);
    let (_, mesh) = &printed[0];
    let (made, report) = edge_engine::remesh::remesh_with_options(
        mesh,
        &edge_engine::remesh::Options {
            voxel_size: 0.5,
            ..Default::default()
        },
        down,
        &|| false,
    )
    .unwrap()
    .unwrap();
    assert!(report.triangles_after > 0);
    model
        .set_part_meshes(Some(&BED), object, vec![(part, made, MeshStat::default())])
        .unwrap();
    assert!(!model.mesh_errors(object, None).unwrap().shown());
    assert!(lowest(&model).abs() < 1e-4);
}
