use std::sync::Arc;

use super::*;
use crate::primitives;

fn cube() -> Mesh {
    Mesh::from_triangle_soup(primitives::cuboid([0.0; 3], [10.0, 10.0, 10.0]))
}

fn selector(mesh: &Mesh) -> Selector {
    Selector::new(mesh, Topology::of(mesh))
}

fn area(corners: &[V3; 3]) -> f32 {
    length(cross(
        sub(corners[1], corners[0]),
        sub(corners[2], corners[0]),
    )) / 2.0
}

/// The area painted with each state.
fn areas(s: &Selector) -> [f32; 4] {
    let mut out = [0.0; 4];
    let mut pieces = Vec::new();
    for at in 0..s.triangle_count() {
        pieces.clear();
        s.leaves(at, &mut pieces);
        for (corners, state, _) in &pieces {
            out[usize::from(*state).min(3)] += area(corners);
        }
    }
    out
}

#[test]
fn codes_read_back_as_written() {
    for code in [
        "4",
        "8",
        "0C",
        "1C",
        "4C",
        "EC",
        "0FC",
        "0FFC",
        "40483",
        "845",
        "44483486",
        "1C0C2C0C1C13",
        "880C2",
    ] {
        let nodes = code::parse(code).unwrap_or_else(|| panic!("{code} not read"));
        assert_eq!(code::write(&nodes), code, "{code}");
    }
    // Lower case is read; written back in capitals.
    assert_eq!(code::write(&code::parse("ec").unwrap()), "EC");
    // What the reference drops.
    for code in ["", "x", "3", "43", "C", "F", "F0C", "34"] {
        assert_eq!(code::parse(code), None, "{code:?}");
    }
    // Whole trees after the first are allowed and not read.
    assert_eq!(code::parse("84"), Some(vec![Node::Leaf(1)]));
    // The highest state, 255: 3 + 16 * 15 + 12.
    let top = code::write(&[Node::Leaf(255)]);
    assert_eq!(code::parse(&top), Some(vec![Node::Leaf(255)]));
}

#[test]
fn a_tree_loaded_is_the_tree_written() {
    let mesh = cube();
    let mut s = selector(&mesh);
    for code in ["4", "8", "0C", "40483", "845", "44483486", "1C0C2C0C1C13"] {
        s.load(3, code, 255);
        assert_eq!(s.code_of(3).as_deref(), Some(code));
        // The pieces are those the code describes, as `edge-3mf` reads them.
        let mut leaves = Vec::new();
        s.leaves(3, &mut leaves);
        let mut read = code::pieces(code, mesh.triangle(3)).unwrap();
        // `leaves` walks children first to last, the code last to first.
        read.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        let mut walked: Vec<([V3; 3], u8)> = leaves.iter().map(|(c, s, _)| (*c, *s)).collect();
        walked.sort_by(|a, b| format!("{a:?}").cmp(&format!("{b:?}")));
        assert_eq!(walked, read, "{code}");
    }
    // A state above the most the tool knows is unpainted.
    s.load(3, "0C", 2);
    assert_eq!(s.code_of(3), None);
}

#[test]
fn neighbours_share_the_corners_of_a_split_side() {
    let mesh = cube();
    let mut s = selector(&mesh);
    // Both triangles of a face split on every side: the side they share
    // is split once, at one corner of both.
    let topology = Topology::of(&mesh);
    let other = topology.neighbors[0]
        .iter()
        .copied()
        .find(|&n| n >= 0)
        .unwrap() as usize;
    s.load(0, "00003", 255);
    let before = s.vertices.len();
    s.load(other, "00003", 255);
    assert_eq!(
        s.vertices.len() - before,
        2,
        "the shared side's middle is reused"
    );
}

#[test]
fn a_sphere_paints_round_its_middle_and_cuts_triangles_to_follow() {
    let mesh = cube();
    let mut s = selector(&mesh);
    // The top face (z = 10): find a triangle on it.
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        2.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    let painted = areas(&s)[1];
    // A disc of radius 2 on the face: about 12.6 mm², the edge limit
    // being 0.05 mm here the cut follows it closely.
    assert!(
        (painted - std::f32::consts::PI * 4.0).abs() < 0.5,
        "{painted}"
    );
    assert!(s.is_painted());
    assert_eq!(s.states(), [1]);
    // Written and read back, it is the same.
    let codes = s.serialize();
    assert!(!codes.is_empty());
    let mut t = selector(&mesh);
    t.load_all(codes.iter().map(|(at, code)| (*at, code.as_str())), 255);
    assert_eq!(t.serialize(), codes);
    assert_eq!(areas(&t), areas(&s));
    // Erased again, it is whole: the pieces are put back together.
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        3.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 0, &Affine::IDENTITY, true, 0.0, false);
    assert!(!s.is_painted());
    assert!(s.serialize().is_empty());
}

#[test]
fn a_circle_paints_only_what_faces_the_camera() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    // A circle larger than the cube, seen from above: the top face only.
    let cursor = Cursor::new(
        Shape::Circle,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        20.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 2, &Affine::IDENTITY, true, 0.0, false);
    let painted = areas(&s)[2];
    assert!((painted - 100.0).abs() < 1e-3, "{painted}");
    // A sphere as large takes the whole cube.
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        20.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 2, &Affine::IDENTITY, true, 0.0, false);
    assert!((areas(&s)[2] - 600.0).abs() < 1e-2);
}

#[test]
fn a_stroke_paints_the_whole_way_along() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    let cursor = Cursor::new(
        Shape::Capsule2D,
        [2.0, 5.0, 10.0],
        [8.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        1.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    // A band 6 mm long and 2 wide with round ends: 12 + π.
    let painted = areas(&s)[1];
    assert!(
        (painted - (12.0 + std::f32::consts::PI)).abs() < 0.4,
        "{painted}"
    );
    let mut t = selector(&mesh);
    let cursor = Cursor::new(
        Shape::Capsule3D,
        [2.0, 5.0, 10.0],
        [8.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        1.0,
        Affine::IDENTITY,
        None,
    );
    t.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    assert!((areas(&t)[1] - painted).abs() < 0.4);
}

#[test]
fn a_scaled_mesh_is_painted_in_the_worlds_millimetres() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    // Twice as large: a sphere of 2 mm in the world is 1 mm on the mesh.
    let world = Affine {
        m: [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]],
        t: [0.0; 3],
    };
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        2.0,
        world,
        None,
    );
    s.select_patch(top, cursor, 1, &world, true, 0.0, false);
    assert!((areas(&s)[1] - std::f32::consts::PI).abs() < 0.2);
    // Stretched along X only, the test is made in the world.
    let mut t = selector(&mesh);
    let world = Affine {
        m: [[2.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        2.0,
        world,
        None,
    );
    t.select_patch(top, cursor, 1, &world, true, 0.0, false);
    // An ellipse of half the disc's area on the mesh.
    assert!(
        (areas(&t)[1] - std::f32::consts::PI * 2.0).abs() < 0.3,
        "{}",
        areas(&t)[1]
    );
}

#[test]
fn a_clipping_plane_keeps_paint_off_what_it_hides() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    // Everything with x > 5 is cut away.
    let clip = Some(Clip {
        normal: [1.0, 0.0, 0.0],
        offset: 5.0,
    });
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        2.0,
        Affine::IDENTITY,
        clip,
    );
    s.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    let painted = areas(&s)[1];
    assert!(
        (painted - std::f32::consts::PI * 2.0).abs() < 0.4,
        "{painted}"
    );
}

#[test]
fn smart_fill_takes_a_face_up_to_its_edges() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    s.seed_fill(
        [5.0, 5.0, 10.0],
        top,
        &Affine::IDENTITY,
        None,
        30.0,
        0.0,
        true,
    );
    assert!(s.has_fill());
    // The face's outline, four sides (each may be split): 40 mm long.
    let contour: f32 = s
        .fill_contour()
        .iter()
        .map(|[a, b]| length(sub(*b, *a)))
        .sum();
    assert!((contour - 40.0).abs() < 1e-3, "{contour}");
    s.apply_fill(1);
    assert!((areas(&s)[1] - 100.0).abs() < 1e-3);
    s.unmark_fill();
    // With an angle of 90 degrees it goes round the corners: everything.
    s.seed_fill(
        [5.0, 5.0, 10.0],
        top,
        &Affine::IDENTITY,
        None,
        90.0,
        0.0,
        true,
    );
    s.apply_fill(2);
    assert!((areas(&s)[2] - 600.0).abs() < 1e-2);
    assert_eq!(s.serialize().len(), 12);
    assert!(s.serialize().iter().all(|(_, code)| code == "8"));
}

#[test]
fn smart_fill_on_overhangs_only_takes_what_faces_down() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let bottom = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 0.0))
        .unwrap();
    s.seed_fill(
        [5.0, 5.0, 0.0],
        bottom,
        &Affine::IDENTITY,
        None,
        90.0,
        45.0,
        true,
    );
    s.apply_fill(1);
    assert!((areas(&s)[1] - 100.0).abs() < 1e-3);
}

#[test]
fn bucket_fill_keeps_to_one_state() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    // A disc of 2 on the top face, then fill from the face beside it: the
    // fill goes round the cube (any angle) but not into the disc.
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        2.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    let disc = areas(&s)[1];
    let side = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[0] == 0.0))
        .unwrap();
    let corners = mesh.triangle(side);
    let middle = scale(add(add(corners[0], corners[1]), corners[2]), 1.0 / 3.0);
    s.bucket_fill(middle, side, None, -1.0, true, true);
    s.apply_fill(2);
    let after = areas(&s);
    assert!((after[1] - disc).abs() < 1e-3);
    assert!((after[2] - (600.0 - disc)).abs() < 1e-2, "{after:?}");
    // "Triangle" (no spreading): only the piece under the pointer.
    s.bucket_fill(middle, side, None, -1.0, false, true);
    s.apply_fill(3);
    let triangle = areas(&s)[3];
    assert!((triangle - area(&corners)).abs() < 1e-3);
}

#[test]
fn a_height_range_paints_a_band() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let cursor = Cursor::new(
        Shape::HeightRange {
            z: 4.0,
            height: 2.0,
        },
        [0.0; 3],
        [0.0; 3],
        [5.0, 5.0, 100.0],
        1.0,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(0, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    // Four sides 10 mm wide, 2 mm high, give or take the tolerance and the
    // edge limit of 0.1 mm.
    let painted = areas(&s)[1];
    assert!((painted - 80.0).abs() < 5.0, "{painted}");
}

#[test]
fn gap_fill_takes_small_islands_into_what_is_round_them() {
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    let cursor = Cursor::new(
        Shape::Sphere,
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 10.0],
        [5.0, 5.0, 100.0],
        0.5,
        Affine::IDENTITY,
        None,
    );
    s.select_patch(top, cursor, 1, &Affine::IDENTITY, true, 0.0, false);
    assert!(areas(&s)[1] > 0.5);
    // An island of about 0.8 mm²: kept under 0.5 mm², filled under 1.
    assert!(s.gaps(0.5).iter().all(|(_, state)| *state != 0) || s.gaps(0.5).is_empty());
    assert!(!s.gaps(1.0).is_empty());
    s.fill_gaps(1.0);
    assert_eq!(areas(&s)[1], 0.0);
}

#[test]
fn what_changed_is_told_once() {
    let mesh = cube();
    let mut s = selector(&mesh);
    s.set_facet(4, 1);
    s.set_facet(2, 1);
    assert_eq!(s.take_changed(), [2, 4]);
    assert!(s.take_changed().is_empty());
    s.reset();
    assert_eq!(s.take_changed(), [2, 4]);
}

#[test]
fn many_strokes_keep_the_tree_sound() {
    // Strokes back and forth, painting and erasing, which makes and drops
    // pieces and corners: what is written reads back the same.
    let mesh = cube();
    let mut s = selector(&mesh);
    let top = (0..mesh.triangles.len())
        .find(|&at| mesh.triangle(at).iter().all(|p| p[2] == 10.0))
        .unwrap();
    for step in 0..40 {
        let x = 1.0 + (step % 9) as f32;
        let y = 1.0 + ((step * 7) % 9) as f32;
        let cursor = Cursor::new(
            Shape::Sphere,
            [x, y, 10.0],
            [x, y, 10.0],
            [x, y, 100.0],
            1.5,
            Affine::IDENTITY,
            None,
        );
        s.select_patch(
            top,
            cursor,
            (step % 3) as u8,
            &Affine::IDENTITY,
            true,
            0.0,
            false,
        );
    }
    let codes = s.serialize();
    let mut t = selector(&mesh);
    t.load_all(codes.iter().map(|(at, code)| (*at, code.as_str())), 255);
    assert_eq!(t.serialize(), codes);
    let total: f32 = areas(&s).iter().sum();
    assert!((total - 600.0).abs() < 1e-2, "{total}");
    let _ = Arc::strong_count(&s.topology);
}
