//! Paint as part of a model: painted, saved, read back; what the slice is
//! given of it; and what each edit of a part does to it, as the reference
//! decides (Model.cpp: `ModelVolume::split` starts each piece unpainted,
//! `ModelVolume::mirror` and a scale only change the volume's
//! transformation and keep it).

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Model, PaintKind, Saving, selector};
use edge_mesh::{Mesh, primitives::cuboid};

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

fn boxes(dir: &Path, name: &str, boxes: &[([f32; 3], [f32; 3])]) -> PathBuf {
    let soup = boxes
        .iter()
        .flat_map(|(at, size)| cuboid(*at, [at[0] + size[0], at[1] + size[1], at[2] + size[2]]));
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join(name);
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    path
}

fn codes(list: &[(u32, &str)]) -> Vec<(u32, String)> {
    list.iter()
        .map(|(at, code)| (*at, (*code).to_owned()))
        .collect()
}

#[test]
fn paint_is_kept_saved_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = boxes(dir.path(), "cube.stl", &[([0.0; 3], [10.0, 10.0, 10.0])]);
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    let part = model.objects()[0].shape().part_ids()[0];
    let before = model.objects()[0].shape().clone();
    let plate_before = model.stamp(None);

    // A seam on two triangles, an enforcer and a blocker split in four.
    let seam = codes(&[(0, "4"), (5, "80443")]);
    assert!(
        model
            .set_paint(Some(&BED), object, PaintKind::Seam, &[(part, seam.clone())])
            .unwrap()
    );
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts()[0].paint_codes(PaintKind::Seam), seam);
    assert_eq!(shape.parts()[0].seam.as_ref().unwrap().len(), 4);
    // The slice is another; what is drawn is not.
    assert_ne!(model.stamp(None), plate_before);
    assert!(std::sync::Arc::ptr_eq(shape.look(), before.look()));
    // The same paint again changes nothing.
    assert!(
        !model
            .set_paint(Some(&BED), object, PaintKind::Seam, &[(part, seam.clone())])
            .unwrap()
    );

    // Colours change what is drawn.
    let colour = codes(&[(2, "8"), (3, "0C")]);
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Color,
            &[(part, colour.clone())],
        )
        .unwrap();
    let painted = model.objects()[0].shape().clone();
    assert!(!std::sync::Arc::ptr_eq(painted.look(), shape.look()));
    assert_eq!(
        painted.parts()[0]
            .paint
            .as_ref()
            .unwrap()
            .iter()
            .filter(|facet| facet.filament > 0)
            .count(),
        2
    );
    assert_eq!(model.place(None).unwrap().painted.len(), 1);

    // Supports and fuzzy skin are kept, and noted as not followed.
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Supports,
            &[(part, codes(&[(7, "4")]))],
        )
        .unwrap();
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::FuzzySkin,
            &[(part, codes(&[(7, "4"), (8, "4")]))],
        )
        .unwrap();
    assert_eq!(
        model.place(None).unwrap().unread_paint,
        ["fuzzy skin", "supports"]
    );
    // All of them on one triangle, in the reference's order.
    let file = dir.path().join("painted.3mf");
    model.save(&file, &Saving::None, Some(&BED)).unwrap();
    let back = Model::open(&file).unwrap();
    let part_back = &back.objects()[0].shape().parts()[0];
    for (kind, expected) in [
        (PaintKind::Seam, seam.clone()),
        (PaintKind::Color, colour.clone()),
        (PaintKind::Supports, codes(&[(7, "4")])),
        (PaintKind::FuzzySkin, codes(&[(7, "4"), (8, "4")])),
    ] {
        assert_eq!(part_back.paint_codes(kind), expected, "{kind:?}");
    }
    let text = {
        use std::io::Read;
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&file).unwrap()).unwrap();
        let at = (0..zip.len())
            .find(|&at| {
                zip.by_index(at)
                    .unwrap()
                    .name()
                    .unwrap()
                    .starts_with("3D/Objects/")
            })
            .unwrap();
        let mut text = String::new();
        zip.by_index(at).unwrap().read_to_string(&mut text).unwrap();
        text
    };
    assert!(
        text.contains(r#" paint_supports="4" paint_fuzzy_skin="4"/>"#),
        "{text}"
    );
    assert_eq!(
        back.place(None).unwrap().unread_paint,
        ["fuzzy skin", "supports"]
    );

    // Erased, it is gone, and so is its note.
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Supports,
            &[(part, Vec::new())],
        )
        .unwrap();
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::FuzzySkin,
            &[(part, Vec::new())],
        )
        .unwrap();
    assert!(model.place(None).unwrap().unread_paint.is_empty());
    assert!(!model.objects()[0].shape().parts()[0].has_paint(PaintKind::Supports));
}

#[test]
fn a_mirror_a_scale_and_a_split_do_to_paint_what_the_reference_does() {
    let dir = tempfile::tempdir().unwrap();
    // Two boxes apart in one mesh, so that its one part can be split.
    let path = boxes(
        dir.path(),
        "two.stl",
        &[
            ([0.0; 3], [10.0, 10.0, 10.0]),
            ([20.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
        ],
    );
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let part = model.objects()[0].shape().part_ids()[0];
    let paint = codes(&[(1, "4"), (13, "8")]);
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Seam,
            &[(part, paint.clone())],
        )
        .unwrap();

    // Mirrored, the copy keeps its paint on the same triangles.
    model.mirror(Some(&BED), &[copy], 0).unwrap();
    assert_eq!(
        model.objects()[0].shape().parts()[0].paint_codes(PaintKind::Seam),
        paint
    );
    model.mirror_parts(Some(&BED), copy, &[part], 1).unwrap();
    assert_eq!(
        model.objects()[0].shape().parts()[0].paint_codes(PaintKind::Seam),
        paint
    );
    // Scaled likewise.
    let (scene, _) = model.in_scene(Some(&BED), copy).unwrap();
    let bigger = edge_engine::Transform::scaling(2.0).then(&scene);
    model.set_transforms(Some(&BED), &[(copy, bigger)]).unwrap();
    assert_eq!(
        model.objects()[0].shape().parts()[0].paint_codes(PaintKind::Seam),
        paint
    );

    // Split to parts: each piece starts unpainted.
    model.split_to_parts(Some(&BED), object).unwrap();
    for piece in model.objects()[0].shape().parts() {
        assert!(piece.paint_codes(PaintKind::Seam).is_empty());
        assert!(piece.seam.is_none());
    }
    // Saved, nothing of the old paint goes onto the pieces.
    let file = dir.path().join("split.3mf");
    model.save(&file, &Saving::None, Some(&BED)).unwrap();
    let back = Model::open(&file).unwrap();
    assert!(
        back.objects()[0]
            .shape()
            .parts()
            .iter()
            .all(|piece| piece.paint_codes(PaintKind::Seam).is_empty())
    );
}

#[test]
fn a_part_split_to_objects_keeps_its_paint() {
    let dir = tempfile::tempdir().unwrap();
    let path = boxes(
        dir.path(),
        "two.stl",
        &[
            ([0.0; 3], [10.0, 10.0, 10.0]),
            ([20.0, 0.0, 0.0], [10.0, 10.0, 10.0]),
        ],
    );
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(Some(&BED), object).unwrap();
    let parts = model.objects()[0].shape().part_ids().to_vec();
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Supports,
            &[(parts[1], codes(&[(3, "8")]))],
        )
        .unwrap();
    // An object of several parts split to objects: each part whole, with
    // its paint (Model.cpp `ModelObject::split` assigns the facets).
    model.split_to_objects(Some(&BED), object).unwrap();
    assert_eq!(model.objects().len(), 2);
    assert!(
        model.objects()[0].shape().parts()[0]
            .paint_codes(PaintKind::Supports)
            .is_empty()
    );
    assert_eq!(
        model.objects()[1].shape().parts()[0].paint_codes(PaintKind::Supports),
        codes(&[(3, "8")])
    );
}

#[test]
fn a_selector_paints_what_the_model_keeps() {
    // The tools' way round: a selector loaded from the part, a stroke, its
    // codes given back.
    let dir = tempfile::tempdir().unwrap();
    let path = boxes(dir.path(), "cube.stl", &[([0.0; 3], [10.0, 10.0, 10.0])]);
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    let part = model.objects()[0].shape().part_ids()[0];
    let mesh = model.objects()[0].shape().parts()[0].mesh.clone();
    let mut s = selector::Selector::new(&mesh, selector::Topology::of(&mesh));
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    let cursor = selector::Cursor::new(
        selector::Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 50.0],
        2.0,
        selector::Affine::IDENTITY,
        None,
    );
    s.select_patch(
        top,
        cursor,
        1,
        &selector::Affine::IDENTITY,
        true,
        0.0,
        false,
    );
    let painted = s.serialize();
    model
        .set_paint(
            Some(&BED),
            object,
            PaintKind::Seam,
            &[(part, painted.clone())],
        )
        .unwrap();
    assert_eq!(
        model.objects()[0].shape().parts()[0].paint_codes(PaintKind::Seam),
        painted
    );
}

/// How long painting takes on a large model: `EDGE_PAINT_BENCH=<project.3mf>
/// cargo test --release -p edge-engine --test paint -- --ignored --nocapture`.
/// What it printed on the Mac mini is in UI_PARITY.md, section 11.
#[test]
#[ignore]
fn timings_on_a_large_model() {
    use std::time::Instant;
    let Ok(path) = std::env::var("EDGE_PAINT_BENCH") else {
        return;
    };
    let model = Model::open(Path::new(&path)).unwrap();
    let object = model
        .objects()
        .iter()
        .max_by_key(|object| {
            object
                .shape()
                .parts()
                .iter()
                .map(|part| part.mesh.triangles.len())
                .sum::<usize>()
        })
        .unwrap();
    let part = &object.shape().parts()[0];
    let mesh = &part.mesh;
    println!(
        "{} triangles, {} vertices",
        mesh.triangles.len(),
        mesh.vertices.len()
    );
    let clock = Instant::now();
    let topology = selector::Topology::of(mesh);
    println!("topology: {:.1} ms", clock.elapsed().as_secs_f64() * 1e3);
    let clock = Instant::now();
    let tree = edge_engine::raycast::Tree::new(mesh);
    println!("tree: {:.1} ms", clock.elapsed().as_secs_f64() * 1e3);
    let clock = Instant::now();
    let mut s = selector::Selector::new(mesh, topology);
    println!("selector: {:.1} ms", clock.elapsed().as_secs_f64() * 1e3);
    let (low, high) = mesh.bounds().unwrap();
    let centre: [f64; 3] = std::array::from_fn(|i| f64::from(low[i] + high[i]) / 2.0);
    // Rays from the front at points across the middle, as a stroke.
    let steps = 400;
    let mut hits = Vec::new();
    let clock = Instant::now();
    for step in 0..steps {
        let x = f64::from(low[0])
            + (f64::from(high[0] - low[0])) * (0.2 + 0.6 * step as f64 / steps as f64);
        let origin = [x, f64::from(low[1]) - 100.0, centre[2]];
        if let Some(hit) = tree.hit(origin, [0.0, 1.0, 0.0], (0.0, f64::INFINITY), &|_| true) {
            hits.push(hit);
        }
    }
    let cast = clock.elapsed().as_secs_f64() * 1e3;
    println!(
        "{} rays, {} hits: {:.3} ms a ray",
        steps,
        hits.len(),
        cast / steps as f64
    );
    let eye = [
        centre[0] as f32,
        (f64::from(low[1]) - 300.0) as f32,
        centre[2] as f32,
    ];
    for (name, radius) in [("small", 1.0f32), ("large", 5.0)] {
        let mut s2 = s.clone();
        let clock = Instant::now();
        let mut worst = 0.0f64;
        for pair in hits.windows(2) {
            let one = Instant::now();
            let a = pair[0].point.map(|c| c as f32);
            let b = pair[1].point.map(|c| c as f32);
            let cursor = selector::Cursor::new(
                selector::Shape::Capsule2D,
                a,
                b,
                eye,
                radius,
                selector::Affine::IDENTITY,
                None,
            );
            s2.select_patch(
                pair[0].triangle,
                cursor,
                1,
                &selector::Affine::IDENTITY,
                true,
                0.0,
                false,
            );
            worst = worst.max(one.elapsed().as_secs_f64() * 1e3);
        }
        let total = clock.elapsed().as_secs_f64() * 1e3;
        println!(
            "stroke ({name}, radius {radius} mm): {:.3} ms a step, {worst:.2} ms at most; {} triangles touched",
            total / (hits.len() - 1) as f64,
            s2.take_changed().len()
        );
        let clock = Instant::now();
        let codes = s2.serialize();
        println!(
            "  serialize: {:.1} ms, {} codes",
            clock.elapsed().as_secs_f64() * 1e3,
            codes.len()
        );
    }
    // A smart fill of everything at 90 degrees, and putting it on.
    let clock = Instant::now();
    let hit = hits[hits.len() / 2];
    s.seed_fill(
        hit.point.map(|c| c as f32),
        hit.triangle,
        &selector::Affine::IDENTITY,
        None,
        30.0,
        0.0,
        true,
    );
    println!(
        "smart fill at 30 degrees: {:.1} ms",
        clock.elapsed().as_secs_f64() * 1e3
    );
    let clock = Instant::now();
    s.apply_fill(1);
    println!("  applied: {:.1} ms", clock.elapsed().as_secs_f64() * 1e3);
}
