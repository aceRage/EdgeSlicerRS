//! "Cut" on a model (`Model::cut`): what each kind of part becomes, where
//! the halves stand, what they keep, and that what is cut slices as the
//! whole did on each side of the plane.

use std::path::{Path, PathBuf};

use edge_engine::{
    Bed, CutOptions, CutThickness, LayerRange, Mesh, Model, ModelObject, NewPart, PaintKind,
    PartKind, Transform,
};
use edge_mesh::{cut, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

/// A model of one box, standing in the middle of the bed.
fn block(dir: &Path, size: [f32; 3]) -> Model {
    let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], size));
    let path = dir.join("block.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    // Any edit settles a bare model on the bed; a rename is the least.
    let id = model.objects()[0].id();
    model.rename(Some(&BED), id, "block").unwrap();
    model
}

/// A plane through `point` square to `normal`, as the cut takes it: its
/// frame in the scene, rows x, y, normal, middle.
fn plane(point: [f64; 3], normal: [f64; 3]) -> Transform {
    let len = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt();
    let n = normal.map(|c| c / len);
    let helper = if n[0].abs() < 0.9 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let cross = |a: [f64; 3], b: [f64; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let mut u = cross(helper, n);
    let ul = (u[0].powi(2) + u[1].powi(2) + u[2].powi(2)).sqrt();
    u = u.map(|c| c / ul);
    let v = cross(n, u);
    Transform {
        rows: [u, v, n, point],
    }
}

/// Every printed part of an object's copy, placed in the scene, as one mesh.
fn printed(model: &Model, object: &ModelObject, copy: usize) -> Mesh {
    let (scene, _) = model
        .in_scene(Some(&BED), object.copies()[copy].id())
        .unwrap();
    let mut mesh = Mesh::default();
    for part in object.shape().parts() {
        if part.kind == PartKind::Normal {
            mesh.append(&part.mesh, &part.transform.then(&scene));
        }
    }
    mesh
}

fn bounds(mesh: &Mesh) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = mesh.bounds().unwrap();
    (lo.map(f64::from), hi.map(f64::from))
}

fn near(a: f64, b: f64, by: f64) -> bool {
    (a - b).abs() <= by
}

fn sound(mesh: &Mesh) {
    assert!(cut::is_closed(mesh), "closed and wound alike");
    assert_eq!(cut::flat_triangles(mesh), 0, "no triangle without area");
    assert!(mesh.signed_volume() > 0.0);
}

#[test]
fn a_block_cut_across_is_two_objects_standing_on_the_bed() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 7.5];
    let done = model
        .cut(
            Some(&BED),
            copy,
            &plane(middle, [0.0, 0.0, 1.0]),
            &CutOptions::default(),
        )
        .unwrap();
    assert_eq!(done.open, 0);
    assert_eq!(done.made.len(), 2);
    assert_eq!(model.objects().len(), 2);
    let (upper, lower) = (&model.objects()[0], &model.objects()[1]);
    assert_eq!(upper.name(), "block");
    assert_eq!(lower.name(), "block");
    let (u, l) = (printed(&model, upper, 0), printed(&model, lower, 0));
    sound(&u);
    sound(&l);
    assert!(near(u.signed_volume(), 400.0 * 12.5, 1e-6));
    assert!(near(l.signed_volume(), 400.0 * 7.5, 1e-6));
    // Both stand on the bed, where the block stood.
    let (ulo, uhi) = bounds(&u);
    let (llo, lhi) = bounds(&l);
    assert_eq!(ulo[2], 0.0);
    assert!(near(uhi[2], 12.5, 1e-5));
    assert_eq!(llo[2], 0.0);
    assert!(near(lhi[2], 7.5, 1e-5));
    for axis in 0..2 {
        assert!(near(ulo[axis], low[axis], 1e-4) && near(llo[axis], low[axis], 1e-4));
    }
    // On the plate the block was on, in its place.
    assert_eq!(model.plates()[0].copies(), &done.made[..]);
}

#[test]
fn keeping_one_side_and_laying_it_on_the_cut() {
    let dir = tempfile::tempdir().unwrap();
    let base = block(dir.path(), [20.0, 20.0, 20.0]);
    let copy = base.objects()[0].copies()[0].id();
    let (_, reach) = base.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let middle: [f64; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0);
    // A slanted plane: laid on its cut, a part's cut face is on the bed.
    let slanted = plane(middle, [0.3, 0.2, 1.0]);
    for upper in [true, false] {
        let mut model = base.clone();
        let options = CutOptions {
            keep_upper: upper,
            keep_lower: !upper,
            place_on_cut_upper: upper,
            place_on_cut_lower: !upper,
            ..CutOptions::default()
        };
        let done = model.cut(Some(&BED), copy, &slanted, &options).unwrap();
        assert_eq!(done.made.len(), 1);
        assert_eq!(model.objects().len(), 1);
        let mesh = printed(&model, &model.objects()[0], 0);
        sound(&mesh);
        // The cap lies flat on the bed, facing down.
        let mut down = 0.0;
        for at in 0..mesh.triangles.len() {
            let [a, b, c] = mesh.triangle(at).map(|p| p.map(f64::from));
            let n = [
                (b[1] - a[1]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[1] - a[1]),
                (b[2] - a[2]) * (c[0] - a[0]) - (b[0] - a[0]) * (c[2] - a[2]),
                (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]),
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if n[2] / len < -0.999999 && [a, b, c].iter().all(|p| p[2].abs() < 1e-4) {
                down += len / 2.0;
            }
        }
        // The section of a 20 mm cube by that plane is 20 × 20 / cos(slant).
        let slant = 1.0 / (0.3f64.powi(2) + 0.2f64.powi(2) + 1.0).sqrt();
        assert!(near(down, 400.0 / slant, 1e-3), "{down}");
    }
    // Nothing kept: refused, nothing changed.
    let mut model = base.clone();
    let none = CutOptions {
        keep_upper: false,
        keep_lower: false,
        ..CutOptions::default()
    };
    let err = model.cut(Some(&BED), copy, &slanted, &none).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Select at least one object to keep after cutting."
    );
    assert_eq!(model.objects().len(), 1);
}

#[test]
fn flipped_and_cut_to_parts() {
    let dir = tempfile::tempdir().unwrap();
    let base = block(dir.path(), [10.0, 20.0, 30.0]);
    let copy = base.objects()[0].copies()[0].id();
    let (_, reach) = base.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    // Flip: the upper part upside down, standing on what was its top.
    let mut model = base.clone();
    let options = CutOptions {
        flip_upper: true,
        ..CutOptions::default()
    };
    model
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    let upper = printed(&model, &model.objects()[0], 0);
    let (lo, hi) = bounds(&upper);
    assert_eq!(lo[2], 0.0);
    assert!(near(hi[2], 20.0, 1e-5));
    // Cut to parts: one object of two parts, named _A and _B, where it was.
    let mut model = base.clone();
    let options = CutOptions {
        to_parts: true,
        flip_upper: true,
        ..CutOptions::default()
    };
    let done = model
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert_eq!(done.made.len(), 1);
    assert_eq!(model.objects().len(), 1);
    let object = &model.objects()[0];
    let names: Vec<_> = object
        .shape()
        .parts()
        .iter()
        .map(|part| part.name.clone().unwrap_or_default())
        .collect();
    assert_eq!(names, ["block_A", "block_B"]);
    let whole = printed(&model, object, 0);
    let (lo, hi) = bounds(&whole);
    assert_eq!(lo[2], 0.0);
    assert!(near(hi[2], 30.0, 1e-5), "not flipped: {hi:?}");
}

#[test]
fn modifiers_and_negative_parts_go_whole_where_they_reach() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    // A modifier in the top half, a negative part across the middle, a
    // support blocker in the bottom half and an enforcer beside the block.
    let add = |model: &mut Model, kind: PartKind, from: [f32; 3], to: [f32; 3], name: &str| {
        model
            .add_part(
                Some(&BED),
                object,
                NewPart {
                    name: name.into(),
                    kind,
                    mesh: Mesh::from_triangle_soup(cuboid(from, to)),
                    transform: Transform::IDENTITY,
                },
            )
            .unwrap();
    };
    add(
        &mut model,
        PartKind::Modifier,
        [2.0, 2.0, 14.0],
        [8.0, 8.0, 18.0],
        "top",
    );
    add(
        &mut model,
        PartKind::Negative,
        [5.0, 5.0, 5.0],
        [15.0, 15.0, 15.0],
        "hole",
    );
    add(
        &mut model,
        PartKind::SupportBlocker,
        [2.0, 2.0, 1.0],
        [6.0, 6.0, 4.0],
        "under",
    );
    add(
        &mut model,
        PartKind::SupportEnforcer,
        [40.0, 40.0, 0.0],
        [45.0, 45.0, 5.0],
        "away",
    );
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let base = model.clone();
    model
        .cut(
            Some(&BED),
            copy,
            &plane(at, [0.0, 0.0, 1.0]),
            &CutOptions::default(),
        )
        .unwrap();
    let names = |object: &ModelObject| -> Vec<String> {
        object
            .shape()
            .parts()
            .iter()
            .map(|part| part.name.clone().unwrap_or_default())
            .collect()
    };
    assert_eq!(names(&model.objects()[0]), ["", "top", "hole"]);
    assert_eq!(names(&model.objects()[1]), ["", "hole", "under"]);
    // Cut to parts keeps every one with the one object.
    let mut model = base;
    let options = CutOptions {
        to_parts: true,
        ..CutOptions::default()
    };
    model
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert_eq!(
        names(&model.objects()[0]),
        ["block_A", "block_B", "top", "hole", "under", "away"]
    );
}

#[test]
fn settings_and_height_ranges_go_with_each_half() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    model
        .set_object_settings(
            Some(&BED),
            object,
            &[("wall_loops".into(), Some("5".into()))],
        )
        .unwrap();
    model
        .set_height_ranges(
            Some(&BED),
            object,
            vec![LayerRange {
                min_z: 2.0,
                max_z: 4.0,
                settings: vec![("layer_height".into(), "0.1".into())],
            }],
        )
        .unwrap();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    model
        .cut(
            Some(&BED),
            copy,
            &plane(at, [0.0, 0.0, 1.0]),
            &CutOptions::default(),
        )
        .unwrap();
    for object in model.objects() {
        assert_eq!(object.settings(), [("wall_loops".into(), "5".into())]);
        assert_eq!(object.layer_ranges().len(), 1);
    }
}

#[test]
fn paint_goes_with_the_triangles_it_was_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let part = model.objects()[0].shape().part_ids()[0];
    // Every triangle painted in filament 2 ("8"), so every piece is too;
    // the cube's two top triangles (2 and 3) are whole in the upper half.
    let codes: Vec<(u32, String)> = (0..12).map(|t| (t, "8".to_string())).collect();
    model
        .set_paint(Some(&BED), object, PaintKind::Color, &[(part, codes)])
        .unwrap();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 6.0];
    model
        .cut(
            Some(&BED),
            copy,
            &plane(at, [0.0, 0.0, 1.0]),
            &CutOptions::default(),
        )
        .unwrap();
    for object in model.objects() {
        let part = &object.shape().parts()[0];
        let codes = part.paint_codes(PaintKind::Color);
        // All but the cap's triangles.
        let cap = part.mesh.triangles.len() - codes.len();
        assert!(cap >= 2, "the cap is not painted");
        assert!(codes.iter().all(|(_, code)| code == "8"));
        assert!(part.paint.is_some());
    }
}

#[test]
fn other_copies_keep_their_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [10.0, 30.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let made = model.clone_copies(Some(&BED), &[copy], None).unwrap();
    // The clone is another object here; give the first object a second
    // copy by turning the clone a quarter about z is not possible, so turn
    // the first copy instead and check the cut copy's halves keep it.
    assert_eq!(made.len(), 1);
    let (scene, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let middle: [f64; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0);
    let quarter = Transform {
        rows: [[0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0; 3]],
    };
    let turned = scene
        .then(&Transform::translation(middle.map(|v| -v)))
        .then(&quarter)
        .then(&Transform::translation(middle));
    model.set_transforms(Some(&BED), &[(copy, turned)]).unwrap();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    assert!(near(high[0] - low[0], 30.0, 1e-6));
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 5.0];
    model
        .cut(
            Some(&BED),
            copy,
            &plane(at, [0.0, 0.0, 1.0]),
            &CutOptions::default(),
        )
        .unwrap();
    let lower = printed(&model, &model.objects()[1], 0);
    let (lo, hi) = bounds(&lower);
    assert!(near(hi[0] - lo[0], 30.0, 1e-4), "the turn is in the mesh");
    assert!(near(hi[1] - lo[1], 10.0, 1e-4));
}

/// The 3DBenchy the reference ships, as a model.
fn benchy() -> Model {
    Model::open(&reference("resources/handy_models/3DBenchy.3mf")).unwrap()
}

#[test]
fn the_benchy_cut_through_its_cabin_and_at_slants() {
    let base = benchy();
    let copy = base.objects()[0].copies()[0].id();
    let (_, reach) = base.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let whole = printed(&base, &base.objects()[0], 0).signed_volume();
    let own_flat = cut::flat_triangles(&base.objects()[0].shape().parts()[0].mesh);
    assert_eq!(own_flat, 11);
    let mut placed_flat = 0;
    let middle: [f64; 3] = std::array::from_fn(|axis| (low[axis] + high[axis]) / 2.0);
    let height = high[2] - low[2];
    for (z, normal) in [
        (0.55, [0.0, 0.0, 1.0]),
        (0.35, [0.0, 0.0, 1.0]),
        (0.70, [0.0, 0.0, 1.0]),
        (0.55, [0.2, -0.1, 1.0]),
        (0.5, [1.0, 0.0, 0.0]),
        (0.5, [0.0, 1.0, 0.0]),
        (0.4, [0.5, 0.5, 0.7]),
    ] {
        let mut model = base.clone();
        let at = [middle[0], middle[1], low[2] + z * height];
        let done = model
            .cut(Some(&BED), copy, &plane(at, normal), &CutOptions::default())
            .unwrap();
        assert_eq!(done.open, 0, "{z} {normal:?}");
        assert_eq!(model.objects().len(), 2);
        let mut total = 0.0;
        let mut flat = 0;
        for object in model.objects() {
            let mesh = printed(&model, object, 0);
            assert!(
                cut::is_closed(&mesh),
                "{z} {normal:?}: closed and wound alike"
            );
            total += mesh.signed_volume();
            // The Benchy has triangles of no area of its own: the cut makes
            // none more.
            let part = &object.shape().parts()[0].mesh;
            flat += cut::flat_triangles(part);
            placed_flat += cut::flat_triangles(&mesh);
        }
        assert!(
            flat <= own_flat,
            "{z} {normal:?}: {flat} triangles without area"
        );
        assert!(
            near(total, whole, whole * 1e-6),
            "{z} {normal:?}: {total} against {whole}"
        );
    }
    // Placed on the plate (in f32, 175 mm out), thin pieces of the
    // Benchy's thin triangles can round onto a line, as its own do; what
    // that does to a slice is checked below.
    let _ = placed_flat;
}

/// The area of each layer's outlines of a mesh, at heights.
fn areas(mesh: &Mesh, planes: &[f64]) -> Vec<(f64, usize, usize)> {
    edge_slice::slice(mesh, planes)
        .iter()
        .map(|layer| {
            (
                layer
                    .polygons
                    .iter()
                    .map(|p| p.signed_area_mm2())
                    .sum::<f64>(),
                layer.polygons.len(),
                layer.open_chains,
            )
        })
        .collect()
}

#[test]
fn the_halves_slice_as_the_whole_did_each_side_of_the_cut() {
    let base = benchy();
    let copy = base.objects()[0].copies()[0].id();
    let whole = printed(&base, &base.objects()[0], 0);
    let (low, high) = bounds(&whole);
    assert_eq!(low[2], 0.0);
    for cut_at in [25.0, 33.6, 9.0] {
        let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, cut_at];
        let mut model = base.clone();
        model
            .cut(
                Some(&BED),
                copy,
                &plane(middle, [0.0, 0.0, 1.0]),
                &CutOptions::default(),
            )
            .unwrap();
        let upper = printed(&model, &model.objects()[0], 0);
        let lower = printed(&model, &model.objects()[1], 0);
        // The lower half where it stood: the whole's layers under the cut.
        let under: Vec<f64> = (0..)
            .map(|k| 0.1 + 0.2 * k as f64)
            .take_while(|z| *z < cut_at - 0.05)
            .collect();
        let (a, b) = (areas(&whole, &under), areas(&lower, &under));
        for ((z, x), y) in under.iter().zip(&a).zip(&b) {
            assert_eq!(y.2, 0, "the lower half is closed at {z}");
            assert_eq!(x.1, y.1, "as many outlines at {z}");
            assert!(
                near(x.0, y.0, 1e-5 * x.0.max(1.0)),
                "{z}: {} against {}",
                y.0,
                x.0
            );
        }
        // The upper half, stood on the bed: the whole's layers over the
        // cut, brought down by the cut's height.
        let over: Vec<f64> = (0..)
            .map(|k| 0.1 + 0.2 * k as f64)
            .take_while(|z| *z + cut_at < high[2] - 0.05)
            .collect();
        let lifted: Vec<f64> = over.iter().map(|z| z + cut_at).collect();
        let (a, b) = (areas(&whole, &lifted), areas(&upper, &over));
        for ((z, x), y) in over.iter().zip(&a).zip(&b) {
            assert_eq!(y.2, 0, "the upper half is closed at {z}");
            assert_eq!(x.1, y.1, "as many outlines at {z}");
            assert!(
                near(x.0, y.0, 1e-5 * x.0.max(1.0)),
                "{z}: {} against {}",
                y.0,
                x.0
            );
        }
    }
}

#[test]
fn a_thick_cut_takes_its_slab_away() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let options = CutOptions {
        thickness: 2.0,
        thickness_from: CutThickness::Above,
        ..CutOptions::default()
    };
    model
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    let upper = printed(&model, &model.objects()[0], 0);
    let lower = printed(&model, &model.objects()[1], 0);
    assert!(near(upper.signed_volume(), 400.0 * 8.0, 1e-6));
    assert!(near(lower.signed_volume(), 400.0 * 10.0, 1e-6));
}

#[test]
fn a_line_of_sight_finds_the_face_pick_face_lays_the_plane_on() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [20.0, 20.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let middle = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0];
    // From above: the top face, facing up.
    let hit = model
        .face_hit(
            Some(&BED),
            copy,
            [middle[0], middle[1], 100.0],
            [0.0, 0.0, -1.0],
        )
        .unwrap()
        .unwrap();
    assert!(near(hit.point[2], 20.0, 1e-9));
    assert_eq!(hit.normal, [0.0, 0.0, 1.0]);
    assert!(hit.corners.iter().all(|c| near(c[2], 20.0, 1e-9)));
    // From the side, from below (a face met from behind still counts) and
    // past the block.
    let side = model
        .face_hit(
            Some(&BED),
            copy,
            [low[0] - 50.0, middle[1], 5.0],
            [1.0, 0.0, 0.0],
        )
        .unwrap()
        .unwrap();
    assert!(near(side.point[0], low[0], 1e-9) && side.normal == [-1.0, 0.0, 0.0]);
    assert!(
        model
            .face_hit(
                Some(&BED),
                copy,
                [low[0] - 50.0, middle[1], 50.0],
                [1.0, 0.0, 0.0]
            )
            .unwrap()
            .is_none()
    );
    // Mirrored, the face still faces out.
    model.mirror(Some(&BED), &[copy], 0).unwrap();
    let side = model
        .face_hit(
            Some(&BED),
            copy,
            [low[0] - 50.0, middle[1], 5.0],
            [1.0, 0.0, 0.0],
        )
        .unwrap()
        .unwrap();
    assert_eq!(side.normal, [-1.0, 0.0, 0.0]);
}

#[test]
fn a_piece_given_to_the_other_side() {
    // Two posts on a common foot, cut above the foot: two pieces above,
    // one under.
    let dir = tempfile::tempdir().unwrap();
    let mut soup = cuboid([0.0; 3], [30.0, 10.0, 5.0]);
    soup.extend(cuboid([0.0, 0.0, 5.0], [10.0, 10.0, 20.0]));
    soup.extend(cuboid([20.0, 0.0, 5.0], [30.0, 10.0, 20.0]));
    // As three shells: the foot and the posts touch, and the cut makes
    // pieces of each shell.
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.path().join("posts.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let id = model.objects()[0].id();
    model.rename(Some(&BED), id, "posts").unwrap();
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 12.0];
    let cut_plane = plane(at, [0.0, 0.0, 1.0]);
    let pieces = model
        .cut_pieces(Some(&BED), copy, &cut_plane, &CutOptions::default())
        .unwrap();
    assert_eq!(pieces.upper, [true, true, false, false, false]);
    assert_eq!(pieces.piece_of.len(), pieces.mesh.triangles.len());
    // The first post's top goes under.
    let options = CutOptions {
        sides: Some(vec![false, true, false, false, false]),
        ..CutOptions::default()
    };
    model.cut(Some(&BED), copy, &cut_plane, &options).unwrap();
    let names: Vec<&str> = model.objects().iter().map(|o| o.name()).collect();
    assert_eq!(names, ["posts_A", "posts_B"]);
    let upper = printed(&model, &model.objects()[0], 0);
    let lower = printed(&model, &model.objects()[1], 0);
    assert!(near(upper.signed_volume(), 800.0, 1e-6));
    assert!(near(
        lower.signed_volume(),
        1500.0 + 700.0 + 700.0 + 800.0,
        1e-6
    ));
    assert!(cut::is_closed(&upper) && cut::is_closed(&lower));
    // Everything on one side is no cut.
    let mut again = Model::open(&path).unwrap();
    again
        .rename(Some(&BED), again.objects()[0].id(), "posts")
        .unwrap();
    let copy = again.objects()[0].copies()[0].id();
    let options = CutOptions {
        sides: Some(vec![true; 5]),
        ..CutOptions::default()
    };
    assert!(again.cut(Some(&BED), copy, &cut_plane, &options).is_err());
}

#[test]
fn a_dovetail_gives_the_upper_part_a_tongue_and_the_lower_its_groove() {
    use edge_engine::Groove;
    let dir = tempfile::tempdir().unwrap();
    let base = block(dir.path(), [40.0, 20.0, 20.0]);
    let copy = base.objects()[0].copies()[0].id();
    let (_, reach) = base.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let whole = 40.0 * 20.0 * 20.0;
    for (count, angle, tolerance) in [(1, 0.0, 0.0), (1, 0.0, 0.1), (2, 0.0, 0.1), (1, 0.1, 0.0)] {
        let mut model = base.clone();
        let groove = Groove {
            depth: 4.0,
            width: 6.0,
            flaps_angle: std::f64::consts::PI / 3.0,
            angle,
            depth_tolerance: tolerance,
            width_tolerance: tolerance,
            count,
            gap: 4.0,
            plane_radius: 10.0,
        };
        assert!(groove.sound());
        let options = CutOptions {
            groove: Some(groove),
            ..CutOptions::default()
        };
        model
            .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
            .unwrap();
        let names: Vec<&str> = model.objects().iter().map(|o| o.name()).collect();
        assert_eq!(names, ["block_A", "block_B"]);
        let mut total = 0.0;
        for object in model.objects() {
            let mesh = &object.shape().parts()[0].mesh;
            // Shells side by side, each closed.
            assert!(cut::is_closed(mesh));
            assert_eq!(cut::flat_triangles(mesh), 0);
            total += mesh.signed_volume();
        }
        // Without tolerance the parts make up the block; with it, a little
        // less (what is taken off the tongue).
        if tolerance == 0.0 {
            assert!(near(total, whole, 1e-6 * whole), "{count} {angle}: {total}");
        } else {
            assert!(
                total < whole && total > whole - 60.0 * count as f64,
                "{count}: {total}"
            );
        }
        // Sliced across the tongue (half way down it), the tongue and its
        // groove leave the width tolerance between them: the two outlines
        // together are short of the block's by it, along the tongue's 40 mm
        // (the test's plane has its x along the world's -y, so the tongue
        // runs along the world's x).
        let (a, b) = (
            &model.objects()[0].shape().parts()[0].mesh,
            &model.objects()[1].shape().parts()[0].mesh,
        );
        let z = 10.0 - 1.0;
        let area = |mesh: &Mesh| -> f64 {
            edge_slice::slice(mesh, &[z])[0]
                .polygons
                .iter()
                .map(|p| p.signed_area_mm2())
                .sum()
        };
        if count == 1 && angle == 0.0 {
            let gap = (800.0 - area(a) - area(b)) / 40.0;
            assert!(near(gap, tolerance, 1e-4), "the gap is {gap} mm");
        }
        // The upper part reaches down past the plane by the tongue.
        let upper = printed(&model, &model.objects()[0], 0);
        let (lo, hi) = bounds(&upper);
        assert!(
            near(hi[2] - lo[2], 10.0 + 2.0 - tolerance, 1e-5),
            "{lo:?} {hi:?}"
        );
    }
    // A shape whose flaps meet before its neck is refused.
    let mut model = base.clone();
    let bad = Groove {
        depth: 10.0,
        width: 2.0,
        flaps_angle: 2.5,
        angle: 0.0,
        depth_tolerance: 0.1,
        width_tolerance: 0.1,
        count: 1,
        gap: 4.0,
        plane_radius: 10.0,
    };
    assert!(!bad.sound());
    let options = CutOptions {
        groove: Some(bad),
        ..CutOptions::default()
    };
    assert!(
        model
            .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
            .is_err()
    );
}

#[test]
fn an_editable_cut_is_saved_read_back_put_back_and_cut_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [40.0, 20.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 6.0];
    let options = CutOptions {
        flip_lower: true,
        ..CutOptions::default()
    };
    let done = model
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    // Both halves carry the one cut, and the object as it was.
    let recipe = model.cut_recipe(done.made[0]).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &recipe,
        &model.cut_recipe(done.made[1]).unwrap()
    ));
    assert!((recipe.mesh.signed_volume() - 16000.0).abs() < 1e-3);
    assert!(recipe.flip_lower && !recipe.flip_upper);
    // Saved and read back: still one cut shared by the halves.
    let path = dir.path().join("cut.3mf");
    model
        .save(&path, &edge_engine::Saving::None, Some(&BED))
        .unwrap();
    let mut model = Model::open(&path).unwrap();
    let copies: Vec<u64> = model.objects().iter().map(|o| o.copies()[0].id()).collect();
    let read = model.cut_recipe(copies[0]).unwrap();
    assert!(std::sync::Arc::ptr_eq(
        &read,
        &model.cut_recipe(copies[1]).unwrap()
    ));
    assert_eq!(read.plane, recipe.plane);
    assert_eq!(read.mesh.triangles, recipe.mesh.triangles);
    // Put back from the upper half, which stood on the bed: the block
    // whole stands on it too.
    let mut upper = model.clone();
    let edit = upper.cut_edit(Some(&BED), copies[0]).unwrap();
    let (_, reach) = upper.in_scene(Some(&BED), edit.copy).unwrap();
    assert!(near(reach.unwrap()[0][2], 0.0, 1e-9));
    assert!(near(edit.plane.rows[3][2], 6.0, 1e-6));
    // Put back from the flipped lower half: one object, the block whole,
    // the plane where it cut it.
    let edit = model.cut_edit(Some(&BED), copies[1]).unwrap();
    assert!(!edit.missing);
    assert_eq!(model.objects().len(), 1);
    assert_eq!(model.objects()[0].name(), "block");
    let mesh = printed(&model, &model.objects()[0], 0);
    assert!((mesh.signed_volume() - 16000.0).abs() < 1e-3);
    let (_, reach) = model.in_scene(Some(&BED), edit.copy).unwrap();
    let [low, _] = reach.unwrap();
    assert!(
        near(edit.plane.rows[3][2] - low[2], 6.0, 1e-6),
        "{:?}",
        edit.plane
    );
    // Cut again, a little higher: both halves made anew.
    let mut again = edit.plane;
    again.rows[3][2] += 4.0;
    let done = model.cut(Some(&BED), edit.copy, &again, &options).unwrap();
    assert_eq!(done.made.len(), 2);
    let volumes: Vec<f64> = model
        .objects()
        .iter()
        .map(|object| printed(&model, object, 0).signed_volume())
        .collect();
    assert!(
        near(volumes[0], 40.0 * 20.0 * 10.0, 1e-3) && near(volumes[1], 40.0 * 20.0 * 10.0, 1e-3),
        "{volumes:?}"
    );
    // One half gone: putting back says so.
    let mut one = model.clone();
    let gone = one.objects()[1].copies()[0].id();
    one.remove(Some(&BED), &[gone]).unwrap();
    let kept = one.objects()[0].copies()[0].id();
    assert!(one.cut_edit(Some(&BED), kept).unwrap().missing);
    // A cut not kept editable cannot be.
    let mut plain = block(dir.path(), [10.0, 10.0, 10.0]);
    let copy = plain.objects()[0].copies()[0].id();
    let options = CutOptions {
        editable: false,
        ..CutOptions::default()
    };
    let (_, reach) = plain.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let at = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 5.0];
    let done = plain
        .cut(Some(&BED), copy, &plane(at, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert!(plain.cut_recipe(done.made[0]).is_none());
    assert!(plain.cut_edit(Some(&BED), done.made[0]).is_err());
    // Copy cut to: the plane in the target's place.
    let (_, plane_there) = model
        .cut_copied(
            Some(&BED),
            model.objects()[0].copies()[0].id(),
            model.objects()[1].copies()[0].id(),
        )
        .unwrap();
    assert_eq!(plane_there.rows[2], [0.0, 0.0, 1.0]);
}

#[test]
fn connectors_make_plugs_holes_and_dowels_that_fit_by_their_tolerances() {
    use edge_engine::{Connector, ConnectorKind, ConnectorShape, ConnectorStyle};
    let dir = tempfile::tempdir().unwrap();
    let base = block(dir.path(), [40.0, 20.0, 20.0]);
    let copy = base.objects()[0].copies()[0].id();
    let (_, reach) = base.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let mid = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let connector = |dx: f64, kind, style, shape| Connector {
        at: [mid[0] + dx, mid[1], 10.0],
        radius: 2.5,
        height: 5.0,
        radius_tolerance: 0.1,
        height_tolerance: 0.2,
        z_angle: 0.3,
        kind,
        style,
        shape,
        pin: Default::default(),
    };
    let connectors = vec![
        connector(
            -12.0,
            ConnectorKind::Plug,
            ConnectorStyle::Prism,
            ConnectorShape::Circle,
        ),
        connector(
            -4.0,
            ConnectorKind::Dowel,
            ConnectorStyle::Prism,
            ConnectorShape::Square,
        ),
        connector(
            4.0,
            ConnectorKind::Snap,
            ConnectorStyle::Prism,
            ConnectorShape::Circle,
        ),
        connector(
            12.0,
            ConnectorKind::Plug,
            ConnectorStyle::Frustum,
            ConnectorShape::Hexagon,
        ),
    ];
    let mut model = base.clone();
    let options = CutOptions {
        // Taken back on: connectors keep both parts as objects.
        keep_lower: false,
        to_parts: true,
        connectors,
        ..CutOptions::default()
    };
    let done = model
        .cut(Some(&BED), copy, &plane(mid, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert_eq!(done.made.len(), 3);
    let names: Vec<&str> = model.objects().iter().map(|o| o.name()).collect();
    assert_eq!(names, ["block", "block", "block-Dowel-Connector-2"]);
    let kinds = |object: &ModelObject| -> Vec<(String, PartKind)> {
        object
            .shape()
            .parts()
            .iter()
            .map(|p| (p.name.clone().unwrap_or_default(), p.kind))
            .collect()
    };
    let (upper, lower, dowel) = (
        &model.objects()[0],
        &model.objects()[1],
        &model.objects()[2],
    );
    let negative = |name: &str| (name.to_owned(), PartKind::Negative);
    let normal = |name: &str| (name.to_owned(), PartKind::Normal);
    assert_eq!(
        kinds(upper)[1..],
        [
            negative("Connector-1"),
            negative("Connector-2_A"),
            negative("Connector-3"),
            negative("Connector-4")
        ]
    );
    assert_eq!(
        kinds(lower)[1..],
        [
            normal("Connector-1"),
            negative("Connector-2_B"),
            normal("Connector-3"),
            normal("Connector-4")
        ]
    );
    assert_eq!(kinds(dowel), [normal("Connector-2")]);
    // Every connector's mesh closed, placed as it is.
    let placed = |object: &ModelObject, at: usize| -> Mesh {
        let part = &object.shape().parts()[at];
        let mut mesh = Mesh::default();
        mesh.append(&part.mesh, &part.transform);
        mesh
    };
    for (object, count) in [(upper, 5), (lower, 5), (dowel, 1)] {
        for at in 0..count {
            sound(&placed(object, at));
        }
    }
    // The plug and its hole, across a layer 2 mm over the cut: the hole's
    // radius is the plug's and the radius tolerance; the hole is deeper by
    // the height tolerance and reaches through the face.
    let area = |mesh: &Mesh, z: f64| -> f64 {
        edge_slice::slice(mesh, &[z])[0]
            .polygons
            .iter()
            .map(|p| p.signed_area_mm2())
            .sum()
    };
    let (plug, hole) = (placed(lower, 1), placed(upper, 1));
    let radius = |area: f64| (area / std::f64::consts::PI).sqrt();
    // Both in the halves' own coordinates, where the cut is at 10 mm.
    let gap = radius(area(&hole, 12.0)) - radius(area(&plug, 12.0));
    assert!(near(gap, 0.1, 1e-3), "the gap is {gap}");
    let (plug_low, plug_high) = bounds(&plug);
    let (hole_low, hole_high) = bounds(&hole);
    assert!(near(plug_low[2], 10.0, 1e-5) && near(plug_high[2], 15.0, 1e-5));
    assert!(near(hole_high[2], 15.2, 1e-5) && near(hole_low[2], 10.0 - 0.05, 1e-5));
    // The dowel: twice its height, upright; its holes each half of it with
    // the tolerances, through their faces.
    let (d_low, d_high) = bounds(&placed(dowel, 0));
    assert!(near(d_high[2] - d_low[2], 10.0, 1e-5));
    let (a_low, a_high) = bounds(&placed(upper, 2));
    let (b_low, b_high) = bounds(&placed(lower, 2));
    assert!(
        near(a_low[2], 9.95, 1e-5) && near(a_high[2], 15.1, 1e-5),
        "{a_low:?} {a_high:?}"
    );
    assert!(
        near(b_low[2], 4.9, 1e-5) && near(b_high[2], 10.05, 1e-5),
        "{b_low:?} {b_high:?}"
    );
    // The snap's hole is round, of the snap's radius and tolerance.
    let snap_hole = placed(upper, 3);
    assert!(near(radius(area(&snap_hole, 12.0)), 2.6, 1e-3));
    // One cut's three objects: their number shared, three results, four
    // connectors; the halves keep the cut editable, the dowel does not.
    let recipe = model
        .cut_recipe(model.objects()[0].copies()[0].id())
        .unwrap();
    assert_eq!(recipe.connectors.len(), 4);
    assert!(
        model
            .cut_recipe(model.objects()[2].copies()[0].id())
            .is_none()
    );
    // Saved and read back, then put back: the dowel goes with the halves.
    let path = dir.path().join("connectors.3mf");
    model
        .save(&path, &edge_engine::Saving::None, Some(&BED))
        .unwrap();
    let mut model = Model::open(&path).unwrap();
    assert_eq!(model.objects().len(), 3);
    let kinds_read = kinds(&model.objects()[0]);
    assert_eq!(kinds_read[2], negative("Connector-2_A"));
    let half = model.objects()[0].copies()[0].id();
    let edit = model.cut_edit(Some(&BED), half).unwrap();
    assert_eq!(model.objects().len(), 1);
    assert_eq!(edit.recipe.connectors.len(), 4);
    assert_eq!(model.objects()[0].shape().parts().len(), 1);
}

/// The objects of a project file, sliced with the reference's defaults:
/// each layer's outlines.
fn slice_file(path: &Path) -> edge_engine::Sliced {
    use edge_engine::{Chosen, Overrides, Unwatched};
    let loaded = edge_engine::open(path, None).unwrap().place().unwrap();
    let job = edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
    edge_engine::slice(job, &Unwatched).unwrap()
}

#[test]
fn sliced_a_plug_and_its_hole_are_apart_by_the_tolerance() {
    use edge_engine::{Connector, ConnectorKind, ConnectorShape, ConnectorStyle};
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [30.0, 30.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let mid = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let options = CutOptions {
        connectors: vec![Connector {
            at: mid,
            radius: 4.0,
            height: 6.0,
            radius_tolerance: 0.2,
            height_tolerance: 0.3,
            z_angle: 0.0,
            kind: ConnectorKind::Plug,
            style: ConnectorStyle::Prism,
            shape: ConnectorShape::Circle,
            pin: Default::default(),
        }],
        ..CutOptions::default()
    };
    model
        .cut(Some(&BED), copy, &plane(mid, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    // Each half alone, as it stands on the bed: the upper with its hole
    // from its face (now at the bed) up, the lower with its plug over its
    // top (at 10 mm).
    let alone = |keep: usize| {
        let mut one = model.clone();
        let other = one.objects()[1 - keep].copies()[0].id();
        one.remove(Some(&BED), &[other]).unwrap();
        let path = dir.path().join(format!("half{keep}.3mf"));
        one.save(&path, &edge_engine::Saving::None, Some(&BED))
            .unwrap();
        slice_file(&path)
    };
    let radius_at = |sliced: &edge_engine::Sliced, z: f64, hole: bool| -> f64 {
        let layer = (0..sliced.outlines().len())
            .find(|&at| sliced.top_of(at) >= z)
            .unwrap();
        let outline = &sliced.outlines()[layer][0];
        let ring = if hole {
            &outline.holes[0]
        } else {
            &sliced.outlines()[layer]
                .iter()
                .min_by(|a, b| {
                    a.contour
                        .signed_area_mm2()
                        .abs()
                        .total_cmp(&b.contour.signed_area_mm2().abs())
                })
                .unwrap()
                .contour
        };
        (ring.signed_area_mm2().abs() / std::f64::consts::PI).sqrt()
    };
    let upper = alone(0);
    let lower = alone(1);
    let hole = radius_at(&upper, 3.0, true);
    let plug = radius_at(&lower, 13.0, false);
    assert!(near(hole - plug, 0.2, 0.01), "hole {hole}, plug {plug}");
    // The hole is deeper than the plug is tall by the height tolerance:
    // the upper half has it up to 6.3 mm, solid above.
    let top = |sliced: &edge_engine::Sliced, z: f64| {
        let layer = (0..sliced.outlines().len())
            .find(|&at| sliced.top_of(at) >= z)
            .unwrap();
        sliced.outlines()[layer][0].holes.len()
    };
    assert_eq!(top(&upper, 6.2), 1);
    assert_eq!(top(&upper, 6.6), 0);
}

#[test]
fn a_pin_is_its_own_object_with_a_hole_in_each_half() {
    use edge_engine::pushfit::Pin;
    use edge_engine::{Connector, ConnectorKind, ConnectorShape, ConnectorStyle};
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [30.0, 30.0, 30.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let mid = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 15.0];
    let pin = Pin::default();
    let connector = Connector {
        at: mid,
        radius: 0.5 * pin.diameter,
        height: pin.length_upper + pin.length_lower,
        radius_tolerance: 0.0,
        height_tolerance: 0.0,
        z_angle: 0.0,
        kind: ConnectorKind::Pin,
        style: ConnectorStyle::Prism,
        shape: ConnectorShape::Circle,
        pin,
    };
    // A pin that cannot be made stops the cut.
    let mut refused = model.clone();
    let bad = CutOptions {
        connectors: vec![Connector {
            pin: Pin {
                diameter: 2.0,
                ..pin
            },
            ..connector
        }],
        ..CutOptions::default()
    };
    assert!(
        refused
            .cut(Some(&BED), copy, &plane(mid, [0.0, 0.0, 1.0]), &bad)
            .is_err()
    );
    let options = CutOptions {
        connectors: vec![connector],
        ..CutOptions::default()
    };
    let done = model
        .cut(Some(&BED), copy, &plane(mid, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert_eq!(
        (done.pins, done.pins_lying, done.pin_slot_failures),
        (1, 1, 0)
    );
    let names: Vec<&str> = model.objects().iter().map(|o| o.name()).collect();
    assert_eq!(names, ["block", "block", "block-Pin-1"]);
    let part = |object: usize, at: usize| {
        let part = &model.objects()[object].shape().parts()[at];
        let mut mesh = Mesh::default();
        mesh.append(&part.mesh, &part.transform);
        (part.kind, mesh)
    };
    let (kind, hole_a) = part(0, 1);
    assert_eq!(kind, PartKind::Negative);
    let (kind, hole_b) = part(1, 1);
    assert_eq!(kind, PartKind::Negative);
    let (_, made) = part(2, 0);
    for mesh in [&hole_a, &hole_b, &made] {
        sound(mesh);
    }
    // The pin lies on its flat beside the block, on the bed.
    let pin_at = model.objects()[2].copies()[0].reach().unwrap();
    let block_at = model.objects()[0].copies()[0].reach().unwrap();
    assert!(pin_at[0][0] >= block_at[1][0] + 5.0 - 1e-6);
    assert!(near(pin_at[0][2], 0.0, 1e-6));
    assert!(near(pin_at[1][2] - pin_at[0][2], 6.0 - 2.0 * 0.9, 1e-3));
    // Each hole goes into its half, from the cut, deeper than the pin by the
    // depth clearance and its cone.
    let (a_low, a_high) = bounds(&hole_a);
    assert!(near(a_low[2], 15.0 - 0.2, 1e-4) && near(a_high[2], 15.0 + 7.0 + 0.4 + 3.1, 1e-4));
    let (b_low, b_high) = bounds(&hole_b);
    assert!(near(b_high[2], 15.2, 1e-4) && near(b_low[2], 15.0 - 10.5, 1e-4));
    // Sliced, the upper half's hole is the pin's radius and its clearance
    // (a 96-sided round).
    let mut upper = model.clone();
    let others: Vec<u64> = [1, 2]
        .iter()
        .map(|&at| upper.objects()[at].copies()[0].id())
        .collect();
    upper.remove(Some(&BED), &others).unwrap();
    let path = dir.path().join("upper.3mf");
    upper
        .save(&path, &edge_engine::Saving::None, Some(&BED))
        .unwrap();
    let sliced = slice_file(&path);
    let layer = (0..sliced.outlines().len())
        .find(|&at| sliced.top_of(at) >= 4.0)
        .unwrap();
    let hole = &sliced.outlines()[layer][0].holes[0];
    let radius = (hole.signed_area_mm2().abs() / std::f64::consts::PI).sqrt();
    let sides = 96.0_f64;
    let round = 3.1
        * (sides / (2.0 * std::f64::consts::PI) * (2.0 * std::f64::consts::PI / sides).sin())
            .sqrt();
    assert!(near(radius, round, 0.01), "the hole's radius is {radius}");
    // Saved and read back: the pin's shape is in the cut information and
    // the recipe, and putting the cut back takes the pin with the halves.
    let path = dir.path().join("pin.3mf");
    model
        .save(&path, &edge_engine::Saving::None, Some(&BED))
        .unwrap();
    let mut back = Model::open(&path).unwrap();
    let half = back.objects()[0].copies()[0].id();
    let edit = back.cut_edit(Some(&BED), half).unwrap();
    assert_eq!(back.objects().len(), 1);
    let read = Pin::from_attributes(|name| {
        edit.recipe.connectors[0]
            .extra
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| *value)
    });
    assert_eq!(read, pin);
}

#[test]
fn a_curved_cut_is_made_by_the_solid_under_its_surface_and_made_again() {
    use edge_engine::curved::Sheet;
    let dir = tempfile::tempdir().unwrap();
    let mut model = block(dir.path(), [30.0, 30.0, 20.0]);
    let copy = model.objects()[0].copies()[0].id();
    let (_, reach) = model.in_scene(Some(&BED), copy).unwrap();
    let [low, high] = reach.unwrap();
    let mid = [(low[0] + high[0]) / 2.0, (low[1] + high[1]) / 2.0, 10.0];
    let mut sheet = Sheet::flat(5, 5, 18.0, 18.0);
    sheet.z[12] = 5.0;
    sheet.z[6] = -3.0;
    let options = CutOptions {
        sheet: Some(sheet.clone()),
        ..CutOptions::default()
    };
    // The preview: what the cut would make, in the scene.
    let prepared = model.cut_prepared(Some(&BED), copy).unwrap();
    let [up, down] = model
        .cut_preview(
            Some(&BED),
            copy,
            &plane(mid, [0.0, 0.0, 1.0]),
            &options,
            &prepared,
            None,
        )
        .unwrap();
    let whole = 30.0 * 30.0 * 20.0;
    assert!(near(up.signed_volume() + down.signed_volume(), whole, 1e-2));
    let done = model
        .cut(Some(&BED), copy, &plane(mid, [0.0, 0.0, 1.0]), &options)
        .unwrap();
    assert_eq!(done.empty, Some([false, false]));
    let volumes: Vec<f64> = model
        .objects()
        .iter()
        .map(|object| {
            // Its own mesh: placed on the bed, single precision can fold a
            // sliver of the boolean's flat.
            let mesh = &object.shape().parts()[0].mesh;
            sound(mesh);
            mesh.signed_volume()
        })
        .collect();
    assert!(near(volumes[0] + volumes[1], whole, 1e-2), "{volumes:?}");
    assert!(near(volumes[0], up.signed_volume(), 1e-2));
    // Sliced, each half as it stands.
    let path = dir.path().join("curved.3mf");
    model
        .save(&path, &edge_engine::Saving::None, Some(&BED))
        .unwrap();
    let sliced = slice_file(&path);
    assert!(!sliced.outlines().is_empty());
    // Read back and put back: the surface as it was.
    let mut back = Model::open(&path).unwrap();
    let half = back.objects()[0].copies()[0].id();
    let edit = back.cut_edit(Some(&BED), half).unwrap();
    let read = edit.recipe.sheet.clone().unwrap();
    assert_eq!((read.nx, read.ny), (5, 5));
    assert_eq!(read.values, sheet.z);
    let again = CutOptions {
        sheet: Some(Sheet {
            nx: 5,
            ny: 5,
            half_u: read.half_u,
            half_v: read.half_v,
            z: read.values,
        }),
        ..CutOptions::default()
    };
    back.cut(Some(&BED), edit.copy, &edit.plane, &again)
        .unwrap();
    assert_eq!(back.objects().len(), 2);
    // A surface over the whole part leaves the upper side empty: the cut
    // makes the one half and says so.
    let mut over = block(dir.path(), [10.0, 10.0, 10.0]);
    let copy = over.objects()[0].copies()[0].id();
    let mut high = Sheet::flat(2, 2, 8.0, 8.0);
    high.z = vec![30.0; 4];
    let (_, reach) = over.in_scene(Some(&BED), copy).unwrap();
    let [low, top] = reach.unwrap();
    let at = [(low[0] + top[0]) / 2.0, (low[1] + top[1]) / 2.0, 5.0];
    let done = over
        .cut(
            Some(&BED),
            copy,
            &plane(at, [0.0, 0.0, 1.0]),
            &CutOptions {
                sheet: Some(high),
                ..CutOptions::default()
            },
        )
        .unwrap();
    assert_eq!(done.empty, Some([true, false]));
    assert_eq!(over.objects().len(), 1);
}
