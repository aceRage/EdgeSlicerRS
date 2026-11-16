//! Text that follows the surface ("Use surface", CutSurface.cpp with
//! EmbossJob.cpp `cut_surface`): on a flat face it is the flat text's
//! shape, on a curved face it keeps to the face, across an edge it covers
//! both faces, inwards for a negative part or a modifier, the layer
//! nearest the text taken where there are several, and it is saved and
//! read back. Each solid is checked closed.

use std::path::Path;

use edge_engine::{
    Bed, Chosen, Model, Overrides, PartKind, Saving, TextConfiguration, Transform, Unwatched,
    emboss::{self, TextFonts},
};
use edge_mesh::{Mesh, primitives::cuboid};
use edge_text::Font;

const BED: Bed = Bed {
    min: [0.0, 0.0],
    max: [200.0, 200.0],
};

type V = [f64; 3];

fn nanum() -> Font {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer/resources/fonts");
    Font::open(&dir.join("NanumGothic-Regular.ttf")).unwrap()
}

fn config(text: &str) -> TextConfiguration {
    TextConfiguration {
        text: text.into(),
        style_name: "NORMAL".into(),
        font_descriptor: "NanumGothic-Regular.ttf".into(),
        descriptor_type: edge_3mf::DescriptorType::FileName,
        face_name: Some("NanumGothic".into()),
        ..TextConfiguration::default()
    }
}

/// Every edge is a side of exactly two triangles, once each way.
fn closed(mesh: &Mesh) -> bool {
    let mut edges = std::collections::HashMap::new();
    for t in &mesh.triangles {
        for k in 0..3 {
            *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
        }
    }
    !mesh.triangles.is_empty()
        && edges
            .iter()
            .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
}

/// A model of one mesh, stood on a plate.
fn model_of(dir: &Path, mesh: &Mesh) -> Model {
    let path = dir.join("part.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    model.add_plate(Some(&BED)).unwrap();
    model
}

fn cube(min: [f32; 3], max: [f32; 3]) -> Mesh {
    Mesh::from_triangle_soup(cuboid(min, max))
}

/// A cylinder standing on Z, `n` sides.
fn cylinder(radius: f64, height: f64, n: usize) -> Mesh {
    let ring: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            [(radius * a.cos()) as f32, (radius * a.sin()) as f32]
        })
        .collect();
    edge_mesh::primitives::prism(&[(ring, Vec::new())], 0.0, height as f32).unwrap()
}

/// A UV sphere about the origin.
fn sphere(radius: f64, seg: usize) -> Mesh {
    let rings = seg / 2;
    let p = |i: usize, j: usize| -> [f32; 3] {
        let th = std::f64::consts::PI * i as f64 / rings as f64;
        let ph = std::f64::consts::TAU * (j % seg) as f64 / seg as f64;
        let (s, z) = (radius * th.sin(), -radius * th.cos());
        [(s * ph.cos()) as f32, (s * ph.sin()) as f32, z as f32]
    };
    let mut soup = Vec::new();
    for i in 0..rings {
        for j in 0..seg {
            let (a, b, c, d) = (p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j));
            if i > 0 {
                soup.push([a, b, c]);
            }
            if i + 1 < rings {
                soup.push([a, c, d]);
            }
        }
    }
    Mesh::from_triangle_soup(soup)
}

struct Placed {
    model: Model,
    object: u64,
    copy: u64,
    /// The object in the scene.
    scene: Transform,
}

fn placed(dir: &Path, mesh: &Mesh) -> Placed {
    let model = model_of(dir, mesh);
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let (scene, _) = model.in_scene(Some(&BED), copy).unwrap();
    Placed {
        model,
        object,
        copy,
        scene,
    }
}

impl Placed {
    /// A point given in the mesh's own coordinates, in the scene.
    fn at(&self, p: V) -> V {
        let part = self.model.objects()[0].shape().parts()[0].transform;
        part.then(&self.scene).apply(p)
    }

    /// The frame of a text at `point` facing `normal`, both in the mesh's
    /// coordinates, and its surface.
    fn frame(&mut self, point: V, normal: V) -> (Transform, emboss::Surface) {
        let part = self.model.objects()[0].shape().parts()[0].transform;
        let world = part.then(&self.scene);
        let tip = world.apply(std::array::from_fn(|i| point[i] + normal[i]));
        let at = world.apply(point);
        let normal = std::array::from_fn(|i| tip[i] - at[i]);
        let frame = self
            .model
            .text_frame_on(Some(&BED), self.copy, (at, normal), None, None)
            .unwrap();
        let surface = self
            .model
            .text_surface(self.object, None, &frame)
            .unwrap()
            .unwrap();
        (frame, surface)
    }

    /// A mesh in the text's frame, in the mesh's own coordinates.
    fn in_part(&self, frame: &Transform, mesh: &Mesh) -> Vec<V> {
        let part = self.model.objects()[0].shape().parts()[0].transform;
        let back = part.inverse().unwrap();
        let to = frame.then(&back);
        mesh.vertices
            .iter()
            .map(|v| to.apply(v.map(f64::from)))
            .collect()
    }
}

fn fonts(font: &Font) -> TextFonts<'_> {
    TextFonts {
        font,
        fallback: None,
    }
}

#[test]
fn on_a_flat_face_it_is_the_flat_texts_shape() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = placed(dir.path(), &cube([0.0; 3], [40.0, 40.0, 20.0]));
    let font = nanum();
    let (_, surface) = p.frame([20.0, 20.0, 20.0], [0.0, 0.0, 1.0]);
    let text = config("Flat");
    let depth = 1.5;
    let flat = emboss::emboss(fonts(&font), &text, depth, PartKind::Normal).unwrap();
    let on =
        emboss::emboss_on_surface(fonts(&font), &text, depth, PartKind::Normal, &surface).unwrap();
    assert!(on.volume.shape.use_surface);
    assert!(closed(&on.mesh));
    // The same outline; the reference's cut runs from 0.015 mm under the
    // face to the full depth over it (`create_emboss_projection`), where
    // the flat text is the depth deep in all.
    let (fl, fh) = flat.mesh.bounds().unwrap();
    let (ol, oh) = on.mesh.bounds().unwrap();
    for i in 0..2 {
        assert!((fl[i] - ol[i]).abs() < 1e-4 && (fh[i] - oh[i]).abs() < 1e-4);
    }
    assert!(
        (ol[2] + 0.015).abs() < 1e-5 && (oh[2] - 1.5).abs() < 1e-5,
        "{ol:?} {oh:?}"
    );
    let ratio = on.mesh.signed_volume() / flat.mesh.signed_volume();
    assert!(
        (ratio - (depth + 0.015) / depth).abs() < 1e-4,
        "{ratio} {}",
        (depth + 0.015) / depth
    );
    // The same bytes again.
    let again =
        emboss::emboss_on_surface(fonts(&font), &text, depth, PartKind::Normal, &surface).unwrap();
    assert_eq!(again.mesh.vertices, on.mesh.vertices);
    assert_eq!(again.mesh.triangles, on.mesh.triangles);
}

/// The distance of a point from the Z axis.
fn radial(v: V) -> f64 {
    (v[0] * v[0] + v[1] * v[1]).sqrt()
}

#[test]
fn on_a_cylinder_it_follows_the_curve() {
    let dir = tempfile::tempdir().unwrap();
    let (radius, depth) = (12.0, 1.0);
    let mut p = placed(dir.path(), &cylinder(radius, 30.0, 128));
    let font = nanum();
    let (frame, surface) = p.frame([0.0, -radius, 15.0], [0.0, -1.0, 0.0]);
    let text = config("WIDE");
    let flat = emboss::emboss(fonts(&font), &text, depth, PartKind::Normal).unwrap();
    let (fl, fh) = flat.mesh.bounds().unwrap();
    // Wider than the cylinder is round at its side: a flat text's ends
    // would stand far off it.
    assert!(fh[0] - fl[0] > 15.0, "{fl:?} {fh:?}");
    let on =
        emboss::emboss_on_surface(fonts(&font), &text, depth, PartKind::Normal, &surface).unwrap();
    assert!(closed(&on.mesh));
    // The polygon's flats lie up to r(1 - cos(π/128)) inside the circle.
    let sag = radius * (1.0 - (std::f64::consts::PI / 128.0).cos());
    let corners = p.in_part(&frame, &on.mesh);
    for v in &corners {
        let r = radial(*v);
        assert!(
            r > radius - sag - 0.015 - 1e-3 && r < radius + depth + 1e-3,
            "{r} at {v:?}"
        );
    }
    // Every part of the shape is there: as much material as the shape
    // times the thickness along the projection, give or take what the
    // curve does to it (less than a tenth here).
    let (shape, scale, _) = emboss::text_shape(fonts(&font), &text).unwrap();
    let area: f64 =
        shape.iter().map(|region| region.area_mm2()).sum::<f64>() * (scale / 1e-6).powi(2);
    let volume = on.mesh.signed_volume();
    let expected = area * (depth + 0.015);
    assert!(
        (volume / expected - 1.0).abs() < 0.1,
        "{volume} against {expected}"
    );
    // Its ends wrap round: the corners furthest from the middle are
    // further back than the middle's.
    let front = corners.iter().map(|v| v[1]).fold(f64::INFINITY, f64::min);
    let back = corners
        .iter()
        .map(|v| v[1])
        .fold(f64::NEG_INFINITY, f64::max);
    assert!(back - front > 3.0, "{front} {back}");
}

#[test]
fn on_a_sphere_it_follows_the_curve() {
    let dir = tempfile::tempdir().unwrap();
    let (radius, depth) = (15.0, 0.8);
    let mut p = placed(dir.path(), &sphere(radius, 96));
    let font = nanum();
    let (frame, surface) = p.frame([0.0, -radius, 0.0], [0.0, -1.0, 0.0]);
    let on = emboss::emboss_on_surface(
        fonts(&font),
        &config("Ball"),
        depth,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    assert!(closed(&on.mesh));
    let corners = p.in_part(&frame, &on.mesh);
    for v in corners {
        let r = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!(
            r > radius - 0.1 && r < radius + depth + 1e-3,
            "{r} at {v:?}"
        );
    }
    assert!(on.mesh.signed_volume() > 0.0);
}

#[test]
fn across_an_edge_it_covers_both_faces() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = placed(dir.path(), &cube([0.0; 3], [40.0, 40.0, 20.0]));
    let font = nanum();
    let depth = 1.0;
    let half = std::f64::consts::FRAC_1_SQRT_2;
    let (frame, surface) = p.frame([20.0, 0.0, 20.0], [0.0, -half, half]);
    let on = emboss::emboss_on_surface(
        fonts(&font),
        &config("EDGE"),
        depth,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    assert!(closed(&on.mesh));
    let corners = p.in_part(&frame, &on.mesh);
    // Moved off each face by the depth along the text's Z: depth/√2 out
    // of the front, up off the top.
    let out = depth * half;
    let on_top = corners.iter().filter(|v| v[2] > 20.0 + out - 1e-3).count();
    let on_front = corners.iter().filter(|v| v[1] < -out + 1e-3).count();
    assert!(on_top > 10 && on_front > 10, "{on_top} {on_front}");
    for v in &corners {
        assert!(v[2] < 20.0 + out + 1e-3 && v[1] > -out - 1e-3, "{v:?}");
        // Nothing inside the block deeper than the 0.015 mm overlap.
        let inside = (v[1] - 0.0).min(20.0 - v[2]);
        assert!(inside < 0.015 * half + 1e-3, "{v:?}");
    }
}

#[test]
fn a_negative_part_and_a_modifier_go_into_the_surface() {
    let dir = tempfile::tempdir().unwrap();
    let (radius, depth) = (12.0, 1.2);
    let mut p = placed(dir.path(), &cylinder(radius, 30.0, 128));
    let font = nanum();
    let (frame, surface) = p.frame([0.0, -radius, 15.0], [0.0, -1.0, 0.0]);
    for kind in [PartKind::Negative, PartKind::Modifier] {
        let on =
            emboss::emboss_on_surface(fonts(&font), &config("CUT"), depth, kind, &surface).unwrap();
        assert!(closed(&on.mesh));
        let sag = radius * (1.0 - (std::f64::consts::PI / 128.0).cos());
        for v in p.in_part(&frame, &on.mesh) {
            let r = radial(v);
            assert!(
                r > radius - sag - depth - 1e-3 && r < radius + 0.015 + 1e-3,
                "{kind:?}: {r} at {v:?}"
            );
        }
    }
    // Added as a negative part and sliced: the object keeps printing.
    let text = emboss::emboss_on_surface(
        fonts(&font),
        &config("CUT"),
        depth,
        PartKind::Negative,
        &surface,
    )
    .unwrap();
    let mesh = text.mesh.clone();
    let scene_point = p.at([0.0, -radius, 15.0]);
    let part = p
        .model
        .add_text(
            Some(&BED),
            p.object,
            p.copy,
            Some((scene_point, [0.0, -1.0, 0.0])),
            None,
            None,
            PartKind::Negative,
            text,
        )
        .unwrap();
    assert_eq!(p.model.text_frame(p.object, part).unwrap(), frame);
    let held = &p.model.objects()[0].shape().parts()[1];
    assert_eq!(held.mesh.triangles, mesh.triangles);
    assert!(sliced(&p.model).contains("G1"));
}

#[test]
fn where_layers_overlap_the_nearest_is_taken() {
    let dir = tempfile::tempdir().unwrap();
    let font = nanum();
    let depth = 1.0;
    // A plate at the back (front face y = 10) and a thinner one in front
    // of it (front face y = 0, 4 mm thick), 6 mm of air between.
    let plates = |front: ([f32; 3], [f32; 3])| {
        let mut both = cube([0.0, 10.0, 0.0], [80.0, 20.0, 30.0]);
        both.append(&cube(front.0, front.1), &Transform::IDENTITY);
        both
    };
    // The front plate as wide as the back: the back plate's face is
    // covered by it everywhere and not used.
    let mut p = placed(dir.path(), &plates(([0.0; 3], [80.0, 4.0, 30.0])));
    let (frame, surface) = p.frame([40.0, 0.0, 15.0], [0.0, -1.0, 0.0]);
    let on = emboss::emboss_on_surface(
        fonts(&font),
        &config("Near"),
        depth,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    assert!(closed(&on.mesh));
    for v in p.in_part(&frame, &on.mesh) {
        assert!(v[1] > -depth - 1e-3 && v[1] < 0.015 + 1e-3, "{v:?}");
    }
    // A front plate narrower than the text: the letters past its ends go
    // onto the back plate, nothing is left between the two.
    let mut p = placed(dir.path(), &plates(([34.0, 0.0, 0.0], [46.0, 4.0, 30.0])));
    let (frame, surface) = p.frame([40.0, 0.0, 15.0], [0.0, -1.0, 0.0]);
    let on = emboss::emboss_on_surface(
        fonts(&font),
        &config("A WIDE ONE"),
        depth,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    assert!(closed(&on.mesh));
    let corners = p.in_part(&frame, &on.mesh);
    let front = corners.iter().filter(|v| v[1] < 0.1).count();
    let back = corners.iter().filter(|v| v[1] > 8.9).count();
    assert!(front > 10 && back > 10, "{front} {back}");
    for v in &corners {
        assert!(v[1] < 0.1 || v[1] > 8.9, "{v:?}");
    }
    // Text that misses the object is refused in the reference's words.
    let mut far = frame;
    far.rows[3][2] += 500.0;
    let off = p.model.text_surface(p.object, None, &far).unwrap().unwrap();
    let missed =
        emboss::emboss_on_surface(fonts(&font), &config("Far"), depth, PartKind::Normal, &off);
    assert_eq!(missed.unwrap_err().to_string(), emboss::NO_SURFACE);
}

#[test]
fn turned_on_the_text_moves_onto_the_surface() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = placed(dir.path(), &cube([0.0; 3], [40.0, 40.0, 20.0]));
    let point = p.at([20.0, 0.0, 10.0]);
    let font = nanum();
    let made = emboss::emboss(fonts(&font), &config("Off"), 1.0, PartKind::Normal).unwrap();
    // 2 mm off the front face ("From surface").
    let part = p
        .model
        .add_text(
            Some(&BED),
            p.object,
            p.copy,
            Some((point, [0.0, -1.0, 0.0])),
            None,
            Some(2.0),
            PartKind::Normal,
            made,
        )
        .unwrap();
    let frame = p.model.text_frame(p.object, part).unwrap();
    let offset = p
        .model
        .text_surface_offset(p.object, part, &frame)
        .unwrap()
        .unwrap();
    assert!(
        offset[0].abs() < 1e-6 && offset[1].abs() < 1e-6 && (offset[2] + 2.0).abs() < 1e-5,
        "{offset:?}"
    );
    let on = emboss::moved_in_frame(&frame, offset);
    assert_eq!(
        p.model.text_surface_offset(p.object, part, &on).unwrap(),
        None
    );
    // The text itself is not its own surface.
    let surface = p
        .model
        .text_surface(p.object, Some(part), &on)
        .unwrap()
        .unwrap();
    let text = emboss::emboss_on_surface(
        fonts(&font),
        &config("Off"),
        1.0,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    let (low, high) = text.mesh.bounds().unwrap();
    assert!((low[2] + 0.015).abs() < 1e-5 && (high[2] - 1.0).abs() < 1e-5);
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

#[test]
fn a_text_on_the_surface_is_saved_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let (radius, depth) = (12.0, 1.0);
    let mut p = placed(dir.path(), &cylinder(radius, 30.0, 96));
    let font = nanum();
    let (_, surface) = p.frame([0.0, -radius, 15.0], [0.0, -1.0, 0.0]);
    let text = emboss::emboss_on_surface(
        fonts(&font),
        &config("Saved"),
        depth,
        PartKind::Normal,
        &surface,
    )
    .unwrap();
    let scene_point = p.at([0.0, -radius, 15.0]);
    let part = p
        .model
        .add_text(
            Some(&BED),
            p.object,
            p.copy,
            Some((scene_point, [0.0, -1.0, 0.0])),
            None,
            None,
            PartKind::Normal,
            text,
        )
        .unwrap();
    let path = dir.path().join("surface.3mf");
    p.model.save(&path, &Saving::None, Some(&BED)).unwrap();
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        std::fs::copy(&path, Path::new(&keep).join("surface.3mf")).unwrap();
    }
    let again = Model::open(&path).unwrap();
    let object = again.objects()[0].id();
    let part_again = again.objects()[0].shape().part_ids()[1];
    let read = again.text_of(object, part_again).unwrap().unwrap();
    assert!(read.shape.use_surface);
    assert_eq!(read.shape.depth, depth);
    let was = &p.model.objects()[0].shape().parts()[1];
    let now = &again.objects()[0].shape().parts()[1];
    assert_eq!(now.mesh.triangles.len(), was.mesh.triangles.len());
    assert!((now.mesh.signed_volume() - was.mesh.signed_volume()).abs() < 1e-3);
    // Made again from what was read, in the frame read: the same solid.
    let frame = again.text_frame(object, part_again).unwrap();
    let saved_frame = p.model.text_frame(p.object, part).unwrap();
    for (a, b) in frame
        .rows
        .iter()
        .flatten()
        .zip(saved_frame.rows.iter().flatten())
    {
        assert!((a - b).abs() < 1e-5, "{frame:?} {saved_frame:?}");
    }
    let surface = again
        .text_surface(object, Some(part_again), &frame)
        .unwrap()
        .unwrap();
    let remade =
        emboss::emboss_on_surface(fonts(&font), &read.text, depth, PartKind::Normal, &surface)
            .unwrap();
    assert!((remade.mesh.signed_volume() - was.mesh.signed_volume()).abs() < 1e-3);
    assert_eq!(sliced(&p.model), sliced(&again));
}

/// The pieces of a mesh that share no corner.
fn pieces(mesh: &Mesh) -> usize {
    let mut parent: Vec<usize> = (0..mesh.vertices.len()).collect();
    fn root(parent: &mut [usize], mut at: usize) -> usize {
        while parent[at] != at {
            parent[at] = parent[parent[at]];
            at = parent[at];
        }
        at
    }
    for t in &mesh.triangles {
        for k in 1..3 {
            let (a, b) = (
                root(&mut parent, t[0] as usize),
                root(&mut parent, t[k] as usize),
            );
            parent[a.max(b)] = a.min(b);
        }
    }
    let used: std::collections::BTreeSet<usize> = mesh
        .triangles
        .iter()
        .map(|t| root(&mut parent, t[0] as usize))
        .collect();
    used.len()
}

/// The reference's own checks of its cut (tests/libslic3r/test_emboss_bend.cpp
/// `check_on_surface`, on its `sphere_below`, `lying_cylinder_below` and
/// `cylinder_cap_below`, all in the text's frame with the text's origin on
/// top): one piece for each letter, every corner of the cut on the target,
/// the cut's area seen along the projection the shape's area to 2e-3,
/// and the solid closed.
#[test]
fn the_references_own_checks_of_the_cut() {
    let font = nanum();
    let text = config("BOX 8");
    let (shape, scale, _) = emboss::text_shape(fonts(&font), &text).unwrap();
    let area: f64 =
        shape.iter().map(|region| region.area_mm2()).sum::<f64>() * (scale / 1e-6).powi(2);
    // Its sphere is of 2° steps, radius 30 mm, its top at the origin.
    let mut ball = sphere(30.0, 180);
    for v in &mut ball.vertices {
        v[2] -= 30.0;
    }
    // A cylinder of radius 25 mm, 120 long, lying along X, its top line at 0.
    let mut lying = cylinder(25.0, 120.0, 180);
    for v in &mut lying.vertices {
        *v = [v[2] - 60.0, -v[1], v[0] - 25.0];
    }
    // A cylinder standing, its top cap at 0.
    let mut cap = cylinder(30.0, 10.0, 180);
    for v in &mut cap.vertices {
        v[2] -= 10.0;
    }
    for (name, target) in [("sphere", &ball), ("lying cylinder", &lying), ("cap", &cap)] {
        assert!(target.signed_volume() > 0.0, "{name}");
        let surface = emboss::Surface::new([(target, Transform::IDENTITY)]).unwrap();
        let depth = 1.0;
        let made =
            emboss::emboss_on_surface(fonts(&font), &text, depth, PartKind::Normal, &surface)
                .unwrap();
        assert!(closed(&made.mesh), "{name}");
        assert_eq!(pieces(&made.mesh), shape.len(), "{name}");
        let tree = edge_mesh::raycast::Tree::new(target);
        let mut seen = 0.0;
        for t in 0..made.mesh.triangles.len() {
            let [a, b, c] = made.mesh.triangle(t).map(|p| p.map(f64::from));
            let along = ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0;
            if along <= 0.0 {
                continue;
            }
            seen += along;
            // The front is the cut moved the depth along Z.
            for p in [a, b, c] {
                let on = [p[0], p[1], p[2] - depth];
                let (q, _, _) = tree.closest_point(on).unwrap();
                let far = ((0..3).map(|i| (q[i] - on[i]).powi(2)).sum::<f64>()).sqrt();
                assert!(far < 1e-3, "{name}: {far} at {on:?}");
            }
        }
        assert!(
            (seen / area - 1.0).abs() < 2e-3,
            "{name}: {seen} against {area}"
        );
    }
}

/// How long a text on the 3DBenchy's hull takes to make again, as on each
/// keystroke: the surface made once for the frame, then the text cut from
/// it for each change. Release build:
/// `cargo test --release -p edge-engine --test emboss_surface -- --ignored --nocapture`
#[test]
#[ignore]
fn timing_on_the_benchy() {
    let bed: Option<&Bed> = None;
    let benchy = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer/resources/handy_models/3DBenchy.3mf");
    let mut model = Model::open(&benchy).unwrap();
    if model.plates().is_empty() {
        model.add_plate(bed).unwrap();
    }
    let object = model.objects()[0].id();
    let copy = model.objects()[0].copies()[0].id();
    let triangles: usize = (model.objects()[0].shape().parts().iter())
        .map(|part| part.mesh.triangles.len())
        .sum();
    let (_, reach) = model.in_scene(bed, copy).unwrap();
    let [low, high] = reach.unwrap();
    // A line of sight at the hull's side, from in front of it.
    let middle = [
        (low[0] + high[0]) / 2.0,
        low[1] - 50.0,
        low[2] + 0.17 * (high[2] - low[2]),
    ];
    let hit = model
        .face_hit(bed, copy, middle, [0.0, 1.0, 0.0])
        .unwrap()
        .unwrap_or_else(|| panic!("no hit from {middle:?} in {low:?} {high:?}"));
    let frame = model
        .text_frame_on(bed, copy, (hit.point, hit.normal), None, None)
        .unwrap();
    let font = nanum();
    let started = std::time::Instant::now();
    let surface = model.text_surface(object, None, &frame).unwrap().unwrap();
    let made_surface = started.elapsed();
    let mut times = Vec::new();
    let mut text = String::new();
    for c in "Benchy 1".chars() {
        text.push(c);
        let started = std::time::Instant::now();
        let made = emboss::emboss_on_surface(
            fonts(&font),
            &config(&text),
            1.0,
            PartKind::Normal,
            &surface,
        );
        times.push(started.elapsed());
        if c != ' ' {
            let mesh = made.unwrap().mesh;
            if !closed(&mesh) {
                let mut edges = std::collections::HashMap::new();
                for t in &mesh.triangles {
                    for k in 0..3 {
                        *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
                    }
                }
                let many = edges.values().filter(|n| **n > 1).count();
                let lonely = edges
                    .keys()
                    .filter(|(a, b)| !edges.contains_key(&(*b, *a)))
                    .count();
                panic!(
                    "{text}: {many} edges more than once, {lonely} without their reverse, of {}",
                    edges.len()
                );
            }
        }
    }
    eprintln!(
        "benchy ({triangles} triangles): surface {made_surface:?}, each text {:?}",
        times
    );
    // For the reference to slice: the text added, a part and a negative
    // part, each saved.
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        for (kind, name) in [
            (PartKind::Normal, "benchy_surface_pos.3mf"),
            (PartKind::Negative, "benchy_surface_neg.3mf"),
        ] {
            let mut model = model.clone();
            let text =
                emboss::emboss_on_surface(fonts(&font), &config("Edge"), 1.0, kind, &surface)
                    .unwrap();
            model
                .add_text(
                    bed,
                    object,
                    copy,
                    Some((hit.point, hit.normal)),
                    None,
                    None,
                    kind,
                    text,
                )
                .unwrap();
            model
                .save(&Path::new(&keep).join(name), &Saving::None, bed)
                .unwrap();
        }
    }
}
