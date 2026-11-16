//! The solid boolean (`edge_mesh::boolean`) on cases whose answers are
//! known: every result closed, consistently wound, with no triangle of
//! zero area and no self-intersection, its volume what inclusion and
//! exclusion say, inside the operands' bounds, and the same bytes on
//! every run.
//!
//! The many random pairs are slow in a debug build and are left out of
//! the default run:
//! `cargo test --release -p edge-mesh --test boolean -- --ignored`
//! (`EDGE_BOOLEAN_CASES` sets how many, `EDGE_BOOLEAN_SEED` where to
//! start; a failure names the seed of its case).

use edge_mesh::boolean::{
    self, BooleanError, InputError, Op, Operand, Options, SelfIntersections, Solid, check,
};
use edge_mesh::{Mesh, Triangle};

type V = [f64; 3];

fn to32(p: V) -> [f32; 3] {
    p.map(|c| c as f32)
}

fn cube(min: V, max: V) -> Mesh {
    Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(to32(min), to32(max)))
}

/// A UV sphere: `seg` around, `seg / 2` from pole to pole.
fn sphere(c: V, r: f64, seg: usize) -> Mesh {
    let rings = seg / 2;
    let p = |i: usize, j: usize| -> [f32; 3] {
        let th = std::f64::consts::PI * i as f64 / rings as f64;
        let ph = std::f64::consts::TAU * (j % seg) as f64 / seg as f64;
        let (s, z) = if i == 0 {
            (0.0, -r)
        } else if i == rings {
            (0.0, r)
        } else {
            (r * th.sin(), -r * th.cos())
        };
        to32([c[0] + s * ph.cos(), c[1] + s * ph.sin(), c[2] + z])
    };
    let mut soup: Vec<Triangle> = Vec::new();
    for i in 0..rings {
        for j in 0..seg {
            let (a, b, cc, d) = (p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j));
            soup.push([a, b, cc]);
            soup.push([a, cc, d]);
        }
    }
    Mesh::from_triangle_soup(soup)
}

/// A cylinder along `axis` round `c`, of radius `r` and half-length `h`.
fn cylinder(c: V, r: f64, h: f64, seg: usize, axis: usize) -> Mesh {
    let place = |x: f64, y: f64, z: f64| -> [f32; 3] {
        let l = match axis {
            0 => [z, x, y],
            1 => [y, z, x],
            _ => [x, y, z],
        };
        to32([c[0] + l[0], c[1] + l[1], c[2] + l[2]])
    };
    let ring = |j: usize, z: f64| {
        let a = std::f64::consts::TAU * (j % seg) as f64 / seg as f64;
        place(r * a.cos(), r * a.sin(), z)
    };
    let (bottom, top) = (place(0.0, 0.0, -h), place(0.0, 0.0, h));
    let mut soup: Vec<Triangle> = Vec::new();
    for j in 0..seg {
        let (a, b, cc, d) = (ring(j, -h), ring(j + 1, -h), ring(j + 1, h), ring(j, h));
        soup.extend([[a, b, cc], [a, cc, d], [bottom, b, a], [top, d, cc]]);
    }
    Mesh::from_triangle_soup(soup)
}

fn torus(c: V, big: f64, small: f64, n: usize, m: usize) -> Mesh {
    let p = |i: usize, j: usize| -> [f32; 3] {
        let u = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        let v = std::f64::consts::TAU * (j % m) as f64 / m as f64;
        let w = big + small * v.cos();
        to32([
            c[0] + w * u.cos(),
            c[1] + w * u.sin(),
            c[2] + small * v.sin(),
        ])
    };
    let mut soup: Vec<Triangle> = Vec::new();
    for i in 0..n {
        for j in 0..m {
            let (a, b, cc, d) = (p(i, j), p(i + 1, j), p(i + 1, j + 1), p(i, j + 1));
            soup.extend([[a, b, cc], [a, cc, d]]);
        }
    }
    Mesh::from_triangle_soup(soup)
}

/// A tetrahedron, wound outward whatever order its corners come in.
fn tetrahedron(p: [V; 4]) -> Mesh {
    let q = p.map(to32);
    let mut soup = vec![
        [q[0], q[2], q[1]],
        [q[0], q[1], q[3]],
        [q[1], q[2], q[3]],
        [q[2], q[0], q[3]],
    ];
    let mesh = Mesh::from_triangle_soup(soup.clone());
    if mesh.signed_volume() < 0.0 {
        for t in &mut soup {
            t.swap(1, 2);
        }
    }
    Mesh::from_triangle_soup(soup)
}

/// A prism: a regular `n`-gon of radius `r` swept `h`, turned by `angles`
/// (radians about x, then y, then z) and moved to `c`.
fn prism(c: V, r: f64, h: f64, n: usize, angles: V) -> Mesh {
    let (sx, cx) = angles[0].sin_cos();
    let (sy, cy) = angles[1].sin_cos();
    let (sz, cz) = angles[2].sin_cos();
    let turn = |p: V| -> [f32; 3] {
        let p = [p[0], cx * p[1] - sx * p[2], sx * p[1] + cx * p[2]];
        let p = [cy * p[0] + sy * p[2], p[1], -sy * p[0] + cy * p[2]];
        let p = [cz * p[0] - sz * p[1], sz * p[0] + cz * p[1], p[2]];
        to32([c[0] + p[0], c[1] + p[1], c[2] + p[2]])
    };
    let ring = |j: usize, z: f64| {
        let a = std::f64::consts::TAU * (j % n) as f64 / n as f64;
        turn([r * a.cos(), r * a.sin(), z])
    };
    let (bottom, top) = (turn([0.0, 0.0, -h / 2.0]), turn([0.0, 0.0, h / 2.0]));
    let mut soup: Vec<Triangle> = Vec::new();
    for j in 0..n {
        let (a, b) = (ring(j, -h / 2.0), ring(j + 1, -h / 2.0));
        let (cc, d) = (ring(j + 1, h / 2.0), ring(j, h / 2.0));
        soup.extend([[a, b, cc], [a, cc, d], [bottom, b, a], [top, d, cc]]);
    }
    Mesh::from_triangle_soup(soup)
}

fn bounds(mesh: &Mesh) -> ([f32; 3], [f32; 3]) {
    mesh.bounds().unwrap_or(([0.0; 3], [0.0; 3]))
}

fn within(inner: &Mesh, lo: [f32; 3], hi: [f32; 3]) -> bool {
    match inner.bounds() {
        None => true,
        Some((a, b)) => (0..3).all(|i| a[i] >= lo[i] && b[i] <= hi[i]),
    }
}

/// The three booleans of `a` and `b`, each checked. Returns their volumes.
/// With `self_check`, each result must not cross itself (or touch itself:
/// the library's exact test does not tell the two apart), in the
/// library's double precision or in single.
fn all_three(name: &str, a: &Mesh, b: &Mesh, self_check: bool) -> [f64; 3] {
    let (volumes, crossings) = all_three_counted(name, a, b, self_check);
    assert_eq!(
        crossings,
        Crossings::default(),
        "{name}: a result crosses itself"
    );
    volumes
}

/// How many of the three results cross or touch themselves.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Crossings {
    /// In the library's double precision.
    double: usize,
    /// In single precision only.
    single: usize,
}

/// [`all_three`], counting the results that cross themselves rather than
/// failing on them.
fn all_three_counted(name: &str, a: &Mesh, b: &Mesh, self_check: bool) -> ([f64; 3], Crossings) {
    let (va, vb) = (a.signed_volume(), b.signed_volume());
    let ((alo, ahi), (blo, bhi)) = (bounds(a), bounds(b));
    let (sa, sb) = (Solid::new(a).unwrap(), Solid::new(b).unwrap());
    let mut volumes = [0.0; 3];
    let mut crossings = Crossings::default();
    for (k, op) in [Op::Union, Op::Difference, Op::Intersection]
        .into_iter()
        .enumerate()
    {
        let solid = sa
            .boolean(&sb, op)
            .unwrap_or_else(|e| panic!("{name} {op:?}: {e}"));
        let result = solid
            .mesh()
            .unwrap_or_else(|e| panic!("{name} {op:?}: {e}"));
        let again = boolean::boolean(a, b, op).unwrap();
        assert_eq!(
            (&result.vertices, &result.triangles),
            (&again.vertices, &again.triangles),
            "{name} {op:?}: not the same twice"
        );
        let defects = check::defects(&result);
        assert!(defects.is_clean(), "{name} {op:?}: {defects:?}");
        if self_check && !result.triangles.is_empty() {
            if solid.crosses_itself() {
                crossings.double += 1;
            } else if check::self_intersects(&result) {
                crossings.single += 1;
            }
        }
        let (lo, hi) = match op {
            Op::Union => (
                std::array::from_fn(|i| alo[i].min(blo[i])),
                std::array::from_fn(|i| ahi[i].max(bhi[i])),
            ),
            Op::Difference => (alo, ahi),
            Op::Intersection => (
                std::array::from_fn(|i| alo[i].max(blo[i])),
                std::array::from_fn(|i| ahi[i].min(bhi[i])),
            ),
        };
        assert!(within(&result, lo, hi), "{name} {op:?}: out of bounds");
        volumes[k] = result.signed_volume();
        assert!(volumes[k] >= -1e-9, "{name} {op:?}: inside out");
    }
    // Inclusion and exclusion, to the single precision of the corners.
    let tolerance = 1e-5 * (va.abs() + vb.abs()) + 1e-9;
    let [u, d, i] = volumes;
    assert!(
        (u - (va + vb - i)).abs() <= tolerance,
        "{name}: |A∪B| {u} against |A|+|B|-|A∩B| {}",
        va + vb - i
    );
    assert!(
        (d - (va - i)).abs() <= tolerance,
        "{name}: |A−B| {d} against |A|-|A∩B| {}",
        va - i
    );
    (volumes, crossings)
}

fn close(got: [f64; 3], want: [f64; 3]) {
    for k in 0..3 {
        assert!(
            (got[k] - want[k]).abs() <= 1e-6 * want[k].abs().max(1.0),
            "{got:?} against {want:?}"
        );
    }
}

#[test]
fn cubes_in_every_relation() {
    let c = cube([0.0; 3], [10.0; 3]);
    let cases: [(&str, Mesh, [f64; 3]); 10] = [
        ("offset", cube([5.0; 3], [15.0; 3]), [1875.0, 875.0, 125.0]),
        ("nested", cube([2.0; 3], [8.0; 3]), [1000.0, 784.0, 216.0]),
        (
            "sharing a face",
            cube([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]),
            [2000.0, 1000.0, 0.0],
        ),
        (
            "on part of a face",
            cube([10.0, 2.0, 2.0], [20.0, 8.0, 8.0]),
            [1360.0, 1000.0, 0.0],
        ),
        (
            "sharing an edge",
            cube([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]),
            [2000.0, 1000.0, 0.0],
        ),
        (
            "touching at a corner",
            cube([10.0; 3], [20.0; 3]),
            [2000.0, 1000.0, 0.0],
        ),
        (
            "identical",
            cube([0.0; 3], [10.0; 3]),
            [1000.0, 0.0, 1000.0],
        ),
        ("apart", cube([20.0; 3], [30.0; 3]), [2000.0, 1000.0, 0.0]),
        (
            "through, flush with two faces",
            cube([2.0, 2.0, 0.0], [8.0, 8.0, 10.0]),
            [1000.0, 640.0, 360.0],
        ),
        (
            "flush with the top",
            cube([2.0, 2.0, 5.0], [8.0, 8.0, 10.0]),
            [1000.0, 820.0, 180.0],
        ),
    ];
    for (name, other, want) in cases {
        close(all_three(name, &c, &other, true), want);
    }
}

#[test]
fn slivers() {
    let c = cube([0.0; 3], [10.0; 3]);
    // A slab that overlaps the cube by a hundredth of a micron.
    let thin = cube([-1.0, -1.0, 9.99999], [11.0, 11.0, 20.0]);
    let v = all_three("thin overlap", &c, &thin, true);
    assert!(
        (v[2] - 100.0 * (10.0 - 9.99999f32 as f64)).abs() < 1e-6,
        "{v:?}"
    );
    // Nearly the same cube.
    let almost = cube([0.00001; 3], [10.0; 3]);
    all_three("almost identical", &c, &almost, true);
}

#[test]
fn cube_with_cylinders_and_spheres() {
    let c = cube([0.0; 3], [10.0; 3]);
    for seg in [8, 32, 128] {
        all_three(
            &format!("sphere {seg}"),
            &c,
            &sphere([10.0; 3], 6.0, seg),
            seg <= 32,
        );
        all_three(
            &format!("sphere inside {seg}"),
            &c,
            &sphere([5.0; 3], 4.0, seg),
            seg <= 32,
        );
        let through = all_three(
            &format!("cylinder {seg}"),
            &c,
            &cylinder([5.0; 3], 3.0, 8.0, seg, 2),
            seg <= 32,
        );
        // Flush with two faces: the hole is the whole cylinder.
        let flush = cylinder([5.0; 3], 3.0, 5.0, seg, 2);
        let v = all_three(&format!("cylinder flush {seg}"), &c, &flush, seg <= 32);
        assert!((v[2] - flush.signed_volume()).abs() < 1e-3, "{v:?}");
        assert!((v[2] - through[2]).abs() < 1e-3);
        assert!((v[0] - 1000.0).abs() < 1e-3);
    }
}

#[test]
fn cylinder_through_cylinder_and_a_torus() {
    let x = cylinder([0.0; 3], 3.0, 10.0, 64, 0);
    let y = cylinder([0.0; 3], 3.0, 10.0, 64, 1);
    let v = all_three("crossed cylinders", &x, &y, true);
    // The Steinmetz solid, 16 r³ / 3, less what the facets cut off.
    assert!((v[2] - 144.0).abs() < 1.0, "{v:?}");
    let z = cylinder([0.0; 3], 2.0, 10.0, 48, 2);
    all_three("cylinders of two sizes", &x, &z, true);
    let t = torus([0.0; 3], 10.0, 3.0, 64, 32);
    let half = cube([0.0, -20.0, -20.0], [20.0, 20.0, 20.0]);
    let v = all_three("torus halved", &t, &half, true);
    assert!((v[1] - v[2]).abs() < 1e-6, "symmetric halves {v:?}");
}

#[test]
fn negative_parts_inside_outside_and_flush() {
    let part = cube([0.0; 3], [20.0, 20.0, 10.0]);
    let inside = cube([5.0, 5.0, 2.0], [15.0, 15.0, 8.0]);
    let r = boolean::difference(&part, &inside).unwrap();
    assert!((r.signed_volume() - (4000.0 - 600.0)).abs() < 1e-9);
    // A void: two shells.
    assert_eq!(edge_mesh::split::split(&r).len(), 2);
    let partly = cube([15.0, 5.0, 2.0], [25.0, 15.0, 8.0]);
    let r = boolean::difference(&part, &partly).unwrap();
    assert!((r.signed_volume() - (4000.0 - 300.0)).abs() < 1e-9);
    let flush = cube([5.0, 5.0, 5.0], [15.0, 15.0, 10.0]);
    let r = boolean::difference(&part, &flush).unwrap();
    assert!((r.signed_volume() - (4000.0 - 500.0)).abs() < 1e-9);
    assert!(check::defects(&r).is_clean());
}

#[test]
fn what_is_refused_and_what_is_put_right() {
    let c = cube([0.0; 3], [10.0; 3]);
    let other = cube([5.0; 3], [15.0; 3]);
    // Empty.
    assert_eq!(
        boolean::union(&Mesh::default(), &c).unwrap_err(),
        (BooleanError::Input {
            operand: Operand::First,
            error: InputError::Empty
        })
    );
    // Open: a face missing.
    let mut open = other.clone();
    open.triangles.truncate(10);
    assert_eq!(
        boolean::difference(&c, &open).unwrap_err(),
        (BooleanError::Input {
            operand: Operand::Second,
            error: InputError::NotClosed
        })
    );
    // Not a number.
    let mut nan = other.clone();
    nan.vertices[0][1] = f32::NAN;
    assert!(matches!(
        boolean::union(&c, &nan),
        Err(BooleanError::Input {
            error: InputError::NotFinite,
            ..
        })
    ));
    // Inside out is turned the right way.
    let mut inverted = other.clone();
    for t in &mut inverted.triangles {
        t.swap(1, 2);
    }
    let r = boolean::difference(&c, &inverted).unwrap();
    assert!((r.signed_volume() - 875.0).abs() < 1e-9);
    // A soup whose corners were never welded is welded.
    let mut soup = Mesh::default();
    for t in 0..other.triangles.len() {
        let corners = other.triangle(t);
        let base = soup.vertices.len() as u32;
        soup.vertices.extend(corners);
        soup.triangles.push([base, base + 1, base + 2]);
    }
    let r = boolean::difference(&c, &soup).unwrap();
    assert!((r.signed_volume() - 875.0).abs() < 1e-9);
}

#[test]
fn shells_that_cross_are_made_one_solid() {
    // Two overlapping cubes in one mesh: it crosses itself.
    let mut two = cube([0.0; 3], [10.0; 3]);
    two.append(&cube([5.0; 3], [15.0; 3]), &edge_mesh::Transform::IDENTITY);
    assert!(check::self_intersects(&two));
    let solid = Solid::new(&two).unwrap();
    assert!(!solid.is_self_intersecting());
    assert!((solid.volume() - 1875.0).abs() < 1e-9);
    let cutter = cube([-1.0, -1.0, 12.0], [20.0, 20.0, 20.0]);
    let r = boolean::difference(&two, &cutter).unwrap();
    assert!((r.signed_volume() - (1875.0 - 3.0 * 100.0)).abs() < 1e-9);
    // Not refused when asked to refuse meshes that cross themselves:
    // made one solid, it does not.
    let options = Options {
        self_intersections: SelfIntersections::Refuse,
        ..Options::default()
    };
    assert!(boolean::boolean_with(&two, &cutter, Op::Difference, &options).is_ok());
}

#[test]
fn a_fold_through_itself_is_refused_when_asked() {
    // One shell, folded through itself: a cube whose top is pushed down
    // through its bottom.
    let mut folded = cube([0.0; 3], [10.0; 3]);
    for v in &mut folded.vertices {
        if v[2] == 10.0 && v[0] == 10.0 && v[1] == 10.0 {
            *v = [10.0, 10.0, -5.0];
        }
    }
    assert!(check::is_closed(&folded));
    assert!(check::self_intersects(&folded));
    assert_eq!(
        Solid::with(&folded, SelfIntersections::Refuse).unwrap_err(),
        InputError::SelfIntersecting
    );
    // Allowed (the default), the result is closed or an error.
    let far = cube([20.0; 3], [30.0; 3]);
    if let Ok(r) = boolean::union(&folded, &far) {
        assert!(check::defects(&r).is_clean());
    }
}

/// A mesh of the triangles of several, its corners at one place made one,
/// as a file of two bodies that touch is read.
fn welded(parts: &[Mesh]) -> Mesh {
    Mesh::from_triangle_soup(parts.iter().flat_map(|m| {
        (0..m.triangles.len())
            .map(|t| m.triangle(t))
            .collect::<Vec<_>>()
    }))
}

/// `mesh` with the edge from `a` to `b` split at `at` (a share of its
/// length) on one side of it only: a T-junction.
fn t_junction(mut mesh: Mesh, a: [f32; 3], b: [f32; 3], at: f64) -> Mesh {
    let find = |m: &Mesh, p: [f32; 3]| m.vertices.iter().position(|v| *v == p).unwrap() as u32;
    let (ia, ib) = (find(&mesh, a), find(&mesh, b));
    let t = mesh
        .triangles
        .iter()
        .position(|t| (0..3).any(|k| t[k] == ia && t[(k + 1) % 3] == ib))
        .unwrap();
    let tr = mesh.triangles[t];
    let k = (0..3).find(|&k| tr[k] == ia).unwrap();
    let v = mesh.vertices.len() as u32;
    mesh.vertices.push(std::array::from_fn(|i| {
        (f64::from(a[i]) + at * (f64::from(b[i]) - f64::from(a[i]))) as f32
    }));
    mesh.triangles[t] = [ia, v, tr[(k + 2) % 3]];
    mesh.triangles.push([v, ib, tr[(k + 2) % 3]]);
    mesh
}

/// Inputs a boolean library finds awkward, as files have them, and what
/// the reference's Manifold refuses and leaves to mcut (MeshBoolean.cpp:
/// `mfd::make_boolean` false on a status other than `NoError`): each gives
/// a closed result whose volumes add up, or is refused for what it is.
#[test]
fn awkward_inputs() {
    let c = cube([0.0; 3], [10.0; 3]);
    // Two bodies that touch along an edge, at a corner, or over a face,
    // read as one mesh with their corners welded: the edge or corner they
    // share belongs to four triangles, the face to two of each body.
    let edge = welded(&[
        cube([0.0; 3], [10.0; 3]),
        cube([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]),
    ]);
    assert!(!check::is_closed(&edge));
    close(
        all_three(
            "touching along an edge",
            &cube([5.0, 5.0, 2.0], [15.0, 15.0, 8.0]),
            &edge,
            true,
        ),
        [2300.0, 300.0, 300.0],
    );
    let corner = welded(&[cube([0.0; 3], [10.0; 3]), cube([10.0; 3], [20.0; 3])]);
    close(
        all_three(
            "touching at a corner",
            &cube([5.0; 3], [15.0; 3]),
            &corner,
            true,
        ),
        [2750.0, 750.0, 250.0],
    );
    let face = welded(&[
        cube([0.0; 3], [10.0; 3]),
        cube([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]),
    ]);
    close(
        all_three(
            "sharing a face",
            &cube([5.0, 2.0, 2.0], [25.0, 8.0, 8.0]),
            &face,
            true,
        ),
        [2180.0, 180.0, 540.0],
    );
    // A shell inside another, both enclosing material, and a shell wound
    // inward inside another (a hollow part).
    let mut nested = cube([0.0; 3], [20.0; 3]);
    nested.append(&cube([5.0; 3], [15.0; 3]), &edge_mesh::Transform::IDENTITY);
    // The inner shell is not material twice over: the solid is the outer.
    let slab = cube([-5.0, -5.0, 8.0], [25.0, 25.0, 12.0]);
    assert!((Solid::new(&nested).unwrap().volume() - 8000.0).abs() < 1e-9);
    for (op, want) in [
        (Op::Union, 10000.0),
        (Op::Difference, 2000.0),
        (Op::Intersection, 1600.0),
    ] {
        let r = boolean::boolean(&slab, &nested, op).unwrap();
        assert!(check::defects(&r).is_clean());
        assert_eq!(edge_mesh::split::split(&r).len(), 1);
        assert!(
            (r.signed_volume() - want).abs() < 1e-9,
            "{op:?} {}",
            r.signed_volume()
        );
    }
    let mut hollow = cube([0.0; 3], [20.0; 3]);
    let mut void = cube([5.0; 3], [15.0; 3]);
    for t in &mut void.triangles {
        t.swap(1, 2);
    }
    hollow.append(&void, &edge_mesh::Transform::IDENTITY);
    close(
        all_three(
            "hollow",
            &hollow,
            &cube([8.0, 8.0, -5.0], [12.0, 12.0, 25.0]),
            true,
        ),
        [7320.0, 6840.0, 160.0],
    );
    // A T-junction, on an edge of the cube and, off the line by the
    // rounding of its corner, on a slanted edge across a face.
    let t = t_junction(c.clone(), [0.0, 0.0, 10.0], [10.0, 0.0, 10.0], 0.5);
    assert_eq!(check::defects(&t).open_edges, 3);
    close(
        all_three("T-junction", &t, &cube([5.0; 3], [15.0; 3]), true),
        [1875.0, 875.0, 125.0],
    );
    let slab = cube([0.0; 3], [10.0, 7.0, 10.0]);
    let (a, b) = (0..slab.triangles.len())
        .map(|t| slab.triangle(t))
        .flat_map(|t| (0..3).map(move |k| (t[k], t[(k + 1) % 3])))
        .find(|(a, b)| a[2] == 10.0 && b[2] == 10.0 && a[0] != b[0] && a[1] != b[1])
        .unwrap();
    let t = t_junction(slab, a, b, 1.0 / 3.0);
    assert!(!check::is_closed(&t));
    close(
        all_three(
            "T-junction off the line",
            &t,
            &cube([5.0; 3], [15.0; 3]),
            true,
        ),
        [1650.0, 650.0, 50.0],
    );
    // A facet written twice.
    let mut twice = cube([5.0; 3], [15.0; 3]);
    twice.triangles.push(twice.triangles[4]);
    for (op, want) in [
        (Op::Union, 1875.0),
        (Op::Difference, 875.0),
        (Op::Intersection, 125.0),
    ] {
        let r = boolean::boolean(&c, &twice, op).unwrap();
        assert!(check::defects(&r).is_clean());
        assert!((r.signed_volume() - want).abs() < 1e-9, "{op:?}");
    }
    // A hole stays refused: nothing says what was there.
    let mut open = cube([5.0; 3], [15.0; 3]);
    open.triangles.truncate(10);
    assert_eq!(
        boolean::union(&c, &open).unwrap_err(),
        BooleanError::Input {
            operand: Operand::Second,
            error: InputError::NotClosed
        }
    );
}

#[test]
fn parts_combined_in_order() {
    let a = cube([0.0; 3], [10.0; 3]);
    let b = cube([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
    let hole = cube([5.0, 2.0, 2.0], [15.0, 8.0, 8.0]);
    let options = Options::default();
    let r = boolean::csg(
        [(Op::Union, &a), (Op::Union, &b), (Op::Difference, &hole)],
        &options,
    )
    .unwrap();
    assert!((r.signed_volume() - (2000.0 - 360.0)).abs() < 1e-9);
    // A negative part before the part it would cut cuts nothing that
    // comes after it, as the reference combines them.
    let r = boolean::csg(
        [(Op::Union, &a), (Op::Difference, &hole), (Op::Union, &b)],
        &options,
    )
    .unwrap();
    assert!((r.signed_volume() - (2000.0 - 180.0)).abs() < 1e-9);
    // A difference with nothing before it does nothing.
    let r = boolean::csg([(Op::Difference, &hole), (Op::Union, &a)], &options).unwrap();
    assert!((r.signed_volume() - 1000.0).abs() < 1e-9);
    // An open part is named.
    let mut open = hole.clone();
    open.triangles.pop();
    let e = boolean::csg([(Op::Union, &a), (Op::Difference, &open)], &options).unwrap_err();
    assert_eq!(e.part, 1);
}

#[test]
fn a_cancelled_boolean_fails() {
    let cancel = boolean::Cancel::new();
    cancel.cancel();
    let options = Options {
        cancel: Some(cancel),
        ..Options::default()
    };
    let r = boolean::boolean_with(
        &sphere([0.0; 3], 5.0, 64),
        &sphere([3.0; 3], 5.0, 64),
        Op::Union,
        &options,
    );
    assert!(matches!(r, Err(BooleanError::Failed(_))), "{r:?}");
}

// --- random pairs

/// xorshift64*, so a case is its seed on every platform.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// On a coarse grid half the time, so faces, edges and corners meet
    /// exactly.
    fn coordinate(&mut self, scale: f64) -> f64 {
        let x = self.unit() * scale;
        if self.next().is_multiple_of(2) {
            (x * 2.0).round() / 2.0
        } else {
            x
        }
    }

    fn point(&mut self, scale: f64) -> V {
        [
            self.coordinate(scale),
            self.coordinate(scale),
            self.coordinate(scale),
        ]
    }

    fn angle(&mut self) -> f64 {
        match self.next() % 4 {
            0 => 0.0,
            1 => std::f64::consts::FRAC_PI_4,
            _ => self.unit() * std::f64::consts::TAU,
        }
    }

    fn shape(&mut self) -> Mesh {
        match self.next() % 3 {
            0 => {
                let (a, b) = (self.point(10.0), self.point(10.0));
                let lo = std::array::from_fn(|i| a[i].min(b[i]));
                let hi = std::array::from_fn(|i| a[i].max(b[i]).max(a[i].min(b[i]) + 0.5));
                cube(lo, hi)
            }
            1 => loop {
                let t = tetrahedron([
                    self.point(10.0),
                    self.point(10.0),
                    self.point(10.0),
                    self.point(10.0),
                ]);
                if t.signed_volume() > 0.5 {
                    break t;
                }
            },
            _ => {
                let n = 3 + (self.next() % 6) as usize;
                let c = self.point(10.0);
                let angles = [self.angle(), self.angle(), self.angle()];
                prism(
                    c,
                    1.0 + self.unit() * 4.0,
                    1.0 + self.unit() * 8.0,
                    n,
                    angles,
                )
            }
        }
    }
}

fn env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "slow: thousands of random pairs; run with --ignored in release"]
fn random_pairs() {
    let cases = env("EDGE_BOOLEAN_CASES", 3000);
    let first = env("EDGE_BOOLEAN_SEED", 1);
    let mut failures = Vec::new();
    // Seeds with a result that crosses or touches itself, in double
    // precision and in single only.
    let (mut double, mut single) = (Vec::new(), Vec::new());
    for seed in first..first + cases {
        let mut random = Random(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
        let a = random.shape();
        // Half the time the second is the first moved or turned onto it,
        // for coplanar faces and shared edges.
        let b = if random.next().is_multiple_of(4) {
            let mut b = a.clone();
            let shift = [0.0, 0.5, 1.0][(random.next() % 3) as usize];
            let axis = (random.next() % 3) as usize;
            for v in &mut b.vertices {
                v[axis] += shift;
            }
            b
        } else {
            random.shape()
        };
        let outcome =
            std::panic::catch_unwind(|| all_three_counted(&format!("seed {seed}"), &a, &b, true));
        if let Ok((_, crossed)) = outcome {
            if crossed.double > 0 {
                double.push(seed);
            } else if crossed.single > 0 {
                single.push(seed);
            }
            continue;
        }
        if let Err(panic) = outcome {
            // `EDGE_BOOLEAN_DUMP=<folder>` keeps the pair as STLs.
            if let Ok(folder) = std::env::var("EDGE_BOOLEAN_DUMP") {
                let folder = std::path::Path::new(&folder);
                for (name, mesh) in [("a", &a), ("b", &b)] {
                    let path = folder.join(format!("seed{seed}_{name}.stl"));
                    std::fs::write(path, edge_mesh::stl::to_binary(mesh)).unwrap();
                }
            }
            let what = (panic.downcast_ref::<String>().cloned())
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            failures.push(format!("seed {seed}: {what}"));
        }
    }
    println!(
        "{cases} pairs from seed {first}: {} failed; a result touches or crosses itself \
         in double precision for {} {double:?}, in single only for {} {single:?}",
        failures.len(),
        double.len(),
        single.len()
    );
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// A solid cut down to where the mesh it came from does not cross
/// itself is no longer marked as crossing itself; its box is the cut's.
#[test]
fn rechecked_after_a_cut_and_its_box() {
    // A cube with a corner pushed through its bottom: one shell that
    // crosses itself; and a clean cube far from it, in the same mesh.
    let mut crossing = cube([0.0; 3], [1.0; 3]);
    for v in &mut crossing.vertices {
        if *v == [1.0, 1.0, 1.0] {
            *v = [0.3, 0.3, -0.6];
        }
    }
    crossing.append(&cube([10.0; 3], [11.0; 3]), &edge_mesh::Transform::IDENTITY);
    let solid = Solid::new(&crossing).unwrap();
    let away = Solid::new(&cube([9.5; 3], [11.5; 3])).unwrap();
    assert!(solid.is_self_intersecting());
    let cut = solid.boolean(&away, Op::Intersection).unwrap();
    assert!(cut.is_self_intersecting());
    let cut = cut.rechecked();
    assert!(!cut.is_self_intersecting());
    assert_eq!(cut.bounds(), Some(([10.0; 3], [11.0; 3])));
}
