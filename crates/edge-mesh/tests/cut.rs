//! Cutting meshes by planes (`edge_mesh::cut`): both halves closed, wound
//! alike, without triangles of no area, their volumes adding up to the
//! whole, on plain and awkward planes.

use edge_mesh::{
    Mesh, Transform,
    cut::{self, Cut},
    primitives,
};

fn cube(size: f32) -> Mesh {
    Mesh::from_triangle_soup(primitives::cuboid([0.0; 3], [size; 3]))
}

/// A closed cylinder of `n` sides, its axis along z from 0 to `height`.
fn cylinder(radius: f32, height: f32, n: usize) -> Mesh {
    let mut soup = Vec::new();
    let at = |k: usize, z: f32| {
        let a = std::f32::consts::TAU * (k % n) as f32 / n as f32;
        [radius * a.cos(), radius * a.sin(), z]
    };
    for k in 0..n {
        let (a0, a1, b0, b1) = (at(k, 0.0), at(k + 1, 0.0), at(k, height), at(k + 1, height));
        soup.push([a0, a1, b1]);
        soup.push([a0, b1, b0]);
        soup.push([[0.0, 0.0, 0.0], a1, a0]);
        soup.push([[0.0, 0.0, height], b0, b1]);
    }
    Mesh::from_triangle_soup(soup)
}

/// A ring: a tube with a hole down its middle, `n` sides.
fn tube(outer: f32, inner: f32, height: f32, n: usize) -> Mesh {
    let mut soup = Vec::new();
    let at = |r: f32, k: usize, z: f32| {
        let a = std::f32::consts::TAU * (k % n) as f32 / n as f32;
        [r * a.cos(), r * a.sin(), z]
    };
    for k in 0..n {
        let (o0, o1, p0, p1) = (
            at(outer, k, 0.0),
            at(outer, k + 1, 0.0),
            at(outer, k, height),
            at(outer, k + 1, height),
        );
        let (i0, i1, j0, j1) = (
            at(inner, k, 0.0),
            at(inner, k + 1, 0.0),
            at(inner, k, height),
            at(inner, k + 1, height),
        );
        soup.extend([[o0, o1, p1], [o0, p1, p0]]);
        soup.extend([[i0, j1, i1], [i0, j0, j1]]);
        soup.extend([[o0, i1, o1], [o0, i0, i1]]);
        soup.extend([[p0, p1, j1], [p0, j1, j0]]);
    }
    Mesh::from_triangle_soup(soup)
}

/// A plane through `point` square to `normal`: takes coordinates to the
/// cut's, the plane `z = 0`, and back.
fn plane(point: [f64; 3], normal: [f64; 3]) -> (Transform, Transform) {
    let len = (normal[0].powi(2) + normal[1].powi(2) + normal[2].powi(2)).sqrt();
    let n = normal.map(|c| c / len);
    // Any two directions square to the normal and to each other.
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
    // Row vectors: a point p goes to p · L + t; the cut's x, y, z are
    // the components along u, v, n.
    let back = Transform {
        rows: [u, v, n, point],
    };
    let to = back.inverse().expect("a plane's frame turns");
    (to, back)
}

fn bounds(mesh: &Mesh) -> ([f64; 3], [f64; 3]) {
    let (lo, hi) = mesh.bounds().expect("not empty");
    (lo.map(f64::from), hi.map(f64::from))
}

/// Checks both halves and answers their volumes.
fn sound(whole: &Mesh, cut: &Cut) -> (f64, f64) {
    assert_eq!(cut.open, 0, "every cut edge is capped");
    for half in [&cut.upper, &cut.lower] {
        if half.is_empty() {
            continue;
        }
        assert!(
            cut::is_closed(&half.mesh),
            "a half is closed and wound alike"
        );
        assert_eq!(
            cut::flat_triangles(&half.mesh),
            0,
            "no triangle without area"
        );
        assert_eq!(half.from.len(), half.mesh.triangles.len());
    }
    let (u, l) = (
        cut.upper.mesh.signed_volume(),
        cut.lower.mesh.signed_volume(),
    );
    assert!(u >= 0.0 && l >= 0.0, "both halves face out: {u} {l}");
    let total = whole.signed_volume();
    assert!(
        (u + l - total).abs() < 1e-6 * total.abs().max(1.0) + 1e-6,
        "the halves make up the whole: {u} + {l} against {total}"
    );
    (u, l)
}

#[test]
fn a_cube_cut_across_the_middle() {
    let mesh = cube(20.0);
    let (to, back) = plane([0.0, 0.0, 7.5], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (u, l) = sound(&mesh, &cut);
    assert!((u - 20.0 * 20.0 * 12.5).abs() < 1e-6);
    assert!((l - 20.0 * 20.0 * 7.5).abs() < 1e-6);
    let (lo, hi) = bounds(&cut.upper.mesh);
    assert_eq!(lo, [0.0, 0.0, 7.5]);
    assert_eq!(hi, [20.0, 20.0, 20.0]);
    let (lo, hi) = bounds(&cut.lower.mesh);
    assert_eq!(lo, [0.0, 0.0, 0.0]);
    assert_eq!(hi, [20.0, 20.0, 7.5]);
    // The side triangles that were not crossed are the cube's own, whole.
    assert!(cut.upper.from.iter().flatten().any(|o| o.whole));
    assert!(cut.upper.from.iter().any(Option::is_none));
}

#[test]
fn planes_through_corners_edges_and_faces() {
    let mesh = cube(10.0);
    // Through the cube's top face, its bottom face, a diagonal through two
    // opposite edges, through corners, and at a slant through one corner.
    let cases: [([f64; 3], [f64; 3]); 7] = [
        ([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ([5.0, 5.0, 5.0], [1.0, -1.0, 0.0]),
        ([5.0, 5.0, 5.0], [1.0, 1.0, 1.0]),
        ([10.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
        ([0.0, 0.0, 0.0], [1.0, 1.0, 1.0]),
        ([5.0, 5.0, 5.0], [0.0, 1.0, 1.0]),
    ];
    for (point, normal) in cases {
        let (to, back) = plane(point, normal);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
    // On a face: everything on one side, nothing on the other.
    let (to, back) = plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    assert!(cut.upper.is_empty());
    assert_eq!(cut.lower.mesh.triangles.len(), 12);
    let (to, back) = plane([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    assert!(cut.lower.is_empty());
    assert_eq!(cut.upper.mesh.triangles.len(), 12);
    // Diagonally through two edges: two prisms of half the cube.
    let (to, back) = plane([5.0, 5.0, 5.0], [1.0, -1.0, 0.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (u, l) = sound(&mesh, &cut);
    assert!((u - 500.0).abs() < 1e-6 && (l - 500.0).abs() < 1e-6);
}

#[test]
fn a_plane_that_misses_leaves_the_mesh_whole() {
    let mesh = cube(10.0);
    let (to, back) = plane([0.0, 0.0, 30.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    assert!(cut.upper.is_empty());
    assert_eq!(cut.lower.mesh.triangles.len(), 12);
    assert_eq!(cut.open, 0);
    assert!(cut::section(&mesh, &to).is_empty());
}

#[test]
fn a_cylinder_at_heights_and_slants() {
    let mesh = cylinder(10.0, 30.0, 64);
    for (point, normal) in [
        ([0.0, 0.0, 12.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 15.0], [0.3, 0.1, 1.0]),
        ([0.0, 0.0, 15.0], [1.0, 0.0, 0.0]),
        ([2.5, -1.0, 15.0], [0.7, 0.7, 0.2]),
        ([0.0, 0.0, 30.0], [0.0, 0.0, 1.0]),
        ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
    ] {
        let (to, back) = plane(point, normal);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
    // Straight across: the upper part is the cylinder's top 18 mm.
    let (to, back) = plane([0.0, 0.0, 12.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (u, l) = sound(&mesh, &cut);
    let whole = mesh.signed_volume();
    assert!((u / whole - 18.0 / 30.0).abs() < 1e-9);
    assert!((l / whole - 12.0 / 30.0).abs() < 1e-9);
}

#[test]
fn a_tube_keeps_its_hole_in_both_caps() {
    let mesh = tube(10.0, 6.0, 20.0, 48);
    for normal in [[0.0, 0.0, 1.0], [0.2, 0.4, 1.0]] {
        let (to, back) = plane([0.0, 0.0, 10.0], normal);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
    // Lengthwise: each side of the tube a piece of its own, in each half.
    let (to, back) = plane([0.0, 0.0, 10.0], [1.0, 0.0, 0.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    sound(&mesh, &cut);
    assert_eq!(edge_mesh::split::split(&cut.upper.mesh).len(), 1);
}

#[test]
fn two_shells_are_cut_each() {
    let mut soup = primitives::cuboid([0.0; 3], [10.0; 3]);
    soup.extend(primitives::cuboid([20.0, 0.0, 0.0], [30.0, 10.0, 20.0]));
    let mesh = Mesh::from_triangle_soup(soup);
    let (to, back) = plane([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (u, l) = sound(&mesh, &cut);
    assert!((u - (500.0 + 1500.0)).abs() < 1e-6);
    assert!((l - (500.0 + 500.0)).abs() < 1e-6);
    assert_eq!(edge_mesh::split::split(&cut.upper.mesh).len(), 2);
    assert_eq!(edge_mesh::split::split(&cut.lower.mesh).len(), 2);
    // Over the lower box: only the taller one is cut.
    let (to, back) = plane([0.0, 0.0, 15.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    sound(&mesh, &cut);
    assert_eq!(edge_mesh::split::split(&cut.upper.mesh).len(), 1);
    assert_eq!(edge_mesh::split::split(&cut.lower.mesh).len(), 2);
}

#[test]
fn a_box_inside_a_box_is_a_hole_in_the_cap() {
    // A hollow box: the inner box wound inside out.
    let mut soup = primitives::cuboid([0.0; 3], [20.0; 3]);
    soup.extend(primitives::inverted(primitives::cuboid(
        [5.0; 3], [15.0; 3],
    )));
    let mesh = Mesh::from_triangle_soup(soup);
    let (to, back) = plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (u, l) = sound(&mesh, &cut);
    assert!((u - 3500.0).abs() < 1e-6 && (l - 3500.0).abs() < 1e-6);
}

#[test]
fn touching_shells_and_a_plane_along_their_seam() {
    // Two boxes sharing an edge line (touching along x = 10, y = 10).
    let mut soup = primitives::cuboid([0.0; 3], [10.0; 3]);
    soup.extend(primitives::cuboid([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]));
    let mesh = Mesh::from_triangle_soup(soup);
    for (point, normal) in [
        ([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]),
        ([10.0, 10.0, 5.0], [1.0, 0.0, 0.0]),
        ([10.0, 10.0, 5.0], [1.0, 1.0, 0.0]),
        ([10.0, 10.0, 5.0], [1.0, -1.0, 0.3]),
    ] {
        let (to, back) = plane(point, normal);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
}

#[test]
fn a_thickness_takes_a_slab_away() {
    let mesh = cube(20.0);
    let (to, back) = plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [-1.0, 1.0], &back);
    assert_eq!(cut.open, 0);
    for half in [&cut.upper, &cut.lower] {
        assert!(cut::is_closed(&half.mesh));
    }
    assert!((cut.upper.mesh.signed_volume() - 400.0 * 9.0).abs() < 1e-6);
    assert!((cut.lower.mesh.signed_volume() - 400.0 * 9.0).abs() < 1e-6);
}

#[test]
fn an_open_mesh_is_left_open_where_it_was() {
    // A cube without its top: cut across, the lower half is capped, the
    // upper half keeps its open rim.
    let soup: Vec<_> = primitives::cuboid([0.0; 3], [10.0; 3])
        .into_iter()
        .enumerate()
        .filter(|(at, _)| !(2..4).contains(at))
        .map(|(_, t)| t)
        .collect();
    let mesh = Mesh::from_triangle_soup(soup);
    let (to, back) = plane([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    assert_eq!(cut.open, 0);
    assert!(cut::is_closed(&cut.lower.mesh));
    assert!(!cut::is_closed(&cut.upper.mesh));
    assert_eq!(cut::flat_triangles(&cut.upper.mesh), 0);
    // A plane along its open rim caps nothing it did not cut.
    let (to, back) = plane([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    assert!(cut.upper.is_empty());
    assert_eq!(cut.lower.mesh.triangles.len(), 10);
    // A flat sheet crossed: nothing to close, one open chain.
    let sheet = Mesh::from_triangle_soup([
        [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 0.0, 10.0]],
        [[0.0, 0.0, 0.0], [10.0, 0.0, 10.0], [0.0, 0.0, 10.0]],
    ]);
    let (to, back) = plane([0.0, 0.0, 5.0], [0.0, 0.0, 1.0]);
    let cut = cut::cut(&sheet, &to, [0.0, 0.0], &back);
    assert!(cut.open >= 1);
    assert_eq!(cut::flat_triangles(&cut.upper.mesh), 0);
    assert_eq!(cut::flat_triangles(&cut.lower.mesh), 0);
}

#[test]
fn many_planes_through_a_fine_cylinder() {
    // Planes at many angles and heights, some through rings of corners.
    let mesh = cylinder(7.0, 21.0, 90);
    let mut checked = 0;
    for i in 0..12 {
        for j in 0..6 {
            let a = i as f64 * 0.52;
            let b = j as f64 * 0.31;
            let normal = [a.cos() * b.sin(), a.sin() * b.sin(), b.cos()];
            let point = [0.3 * i as f64 - 1.5, 0.0, 1.5 + 3.0 * j as f64];
            let (to, back) = plane(point, normal);
            let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
            sound(&mesh, &cut);
            checked += 1;
        }
    }
    assert_eq!(checked, 72);
}

#[test]
fn halves_come_back_where_back_puts_them() {
    let mesh = cube(10.0);
    let (to, _) = plane([0.0, 0.0, 4.0], [0.0, 0.0, 1.0]);
    // Back into the cube's own coordinates, then moved 100 mm along x:
    // the halves stand there.
    let back = to
        .inverse()
        .unwrap()
        .then(&Transform::translation([100.0, 0.0, 0.0]));
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
    let (lo, hi) = bounds(&cut.lower.mesh);
    assert_eq!(lo, [100.0, 0.0, 0.0]);
    assert_eq!(hi, [110.0, 10.0, 4.0]);
    // A mirroring map back keeps them solids.
    let mut mirror = back;
    mirror.rows[0] = mirror.rows[0].map(|c| -c);
    let cut = cut::cut(&mesh, &to, [0.0, 0.0], &mirror);
    sound(&mesh, &cut);
}

/// A sphere of `rings` bands and `sides` sides, poles included.
fn sphere(radius: f64, rings: usize, sides: usize) -> Mesh {
    let at = |ring: usize, side: usize| -> [f32; 3] {
        if ring == rings {
            return [0.0, 0.0, -radius as f32];
        }
        let polar = std::f64::consts::PI * ring as f64 / rings as f64;
        let around = std::f64::consts::TAU * (side % sides) as f64 / sides as f64;
        [
            (radius * polar.sin() * around.cos()) as f32,
            (radius * polar.sin() * around.sin()) as f32,
            (radius * polar.cos()) as f32,
        ]
    };
    let mut soup = Vec::new();
    for ring in 0..rings {
        for side in 0..sides {
            let (a, b, c, d) = (
                at(ring, side),
                at(ring, side + 1),
                at(ring + 1, side + 1),
                at(ring + 1, side),
            );
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

#[test]
fn a_sphere_on_planes_at_random_and_through_its_rings() {
    let mesh = sphere(12.0, 24, 40);
    assert!(cut::is_closed(&mesh));
    // A small generator of its own, so the planes are the same every run.
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..200 {
        let normal = [next() - 0.5, next() - 0.5, next() - 0.5];
        let point = [
            next() * 16.0 - 8.0,
            next() * 16.0 - 8.0,
            next() * 16.0 - 8.0,
        ];
        let (to, back) = plane(point, normal);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
    // Straight across at the height of each ring of corners.
    for ring in 1..24 {
        let z = 12.0 * (std::f64::consts::PI * ring as f64 / 24.0).cos();
        let (to, back) = plane([0.0, 0.0, f64::from(z as f32)], [0.0, 0.0, 1.0]);
        let cut = cut::cut(&mesh, &to, [0.0, 0.0], &back);
        sound(&mesh, &cut);
    }
}
