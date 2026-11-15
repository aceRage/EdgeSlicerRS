//! The reference's own tests of the collapse
//! (tests/libslic3r/test_indexed_triangle_set.cpp), and the results the
//! reference's code gives on the same meshes, to the bit.

use std::path::{Path, PathBuf};

use super::*;
use crate::{Triangle, raw};

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer/tests/data")
        .join(name)
}

fn run(mesh: &mut Mesh, count: u32, max_error: Option<f32>) -> Stats {
    quadric_edge_collapse(mesh, count, max_error, &mut |_| {}, &|| false).expect("not cancelled")
}

/// `its_make_cube` (TriangleMesh.cpp l.886).
fn its_make_cube(x: f32, y: f32, z: f32) -> Mesh {
    Mesh {
        vertices: vec![
            [x, y, 0.0],
            [x, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, y, 0.0],
            [x, y, z],
            [0.0, y, z],
            [0.0, 0.0, z],
            [x, 0.0, z],
        ],
        triangles: vec![
            [0, 1, 2],
            [0, 2, 3],
            [4, 5, 6],
            [4, 6, 7],
            [0, 4, 7],
            [0, 7, 1],
            [1, 7, 6],
            [1, 6, 2],
            [2, 6, 5],
            [2, 5, 3],
            [4, 0, 3],
            [4, 3, 5],
        ],
    }
}

fn point_triangle_distance2(p: [f32; 3], t: Triangle) -> f64 {
    // Ericson, Real-Time Collision Detection 5.1.5, in f64.
    let p = p.map(f64::from);
    let [a, b, c] = t.map(|v| v.map(f64::from));
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let at = |q: V3| dot(sub(p, q), sub(p, q));
    if d1 <= 0.0 && d2 <= 0.0 {
        return at(a);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return at(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return at([a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return at(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return at([a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return at([b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]]);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    at([
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ])
}

/// `is_similar`: the corners and the middles of `to`'s triangles, each's
/// distance to `from`; the largest and the mean within the bounds.
fn is_similar(from: &Mesh, to: &Mesh, max_average: f32, max_distance: f32) -> bool {
    let tris: Vec<Triangle> = (0..from.triangles.len())
        .map(|i| from.triangle(i))
        .collect();
    let mut points = to.vertices.clone();
    for t in &to.triangles {
        let mut c = [0.0f32; 3];
        for &v in t {
            for (sum, x) in c.iter_mut().zip(to.vertices[v as usize]) {
                *sum += x / 3.0;
            }
        }
        points.push(c);
    }
    let (mut sum, mut max) = (0.0f64, 0.0f64);
    for p in &points {
        let d = tris
            .iter()
            .map(|&t| point_triangle_distance2(*p, t))
            .fold(f64::MAX, f64::min)
            .sqrt();
        sum += d;
        max = max.max(d);
    }
    sum / points.len() as f64 <= f64::from(max_average) && max <= f64::from(max_distance)
}

#[test]
fn reduce_one_edge_by_quadric_edge_collapse() {
    let original = Mesh {
        vertices: vec![
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.9, 0.1, -0.1],
        ],
        triangles: vec![
            [1, 0, 3],
            [2, 1, 3],
            [0, 2, 3],
            [0, 1, 4],
            [1, 2, 4],
            [2, 0, 4],
        ],
    };
    let mut mesh = original.clone();
    run(&mut mesh, original.triangles.len() as u32 - 1, None);
    assert_eq!(mesh.triangles.len(), 4);
    assert_eq!(mesh.vertices.len(), 4);
    assert_eq!(mesh.triangles[..3], original.triangles[..3]);
    for i in [0, 1, 3] {
        assert_eq!(mesh.vertices[i], original.vertices[i]);
    }
    let (v, v2, v4) = (mesh.vertices[2], original.vertices[2], original.vertices[4]);
    for i in 0..3 {
        assert!((v[i] < v4[i] && v[i] > v2[i]) || (v[i] > v4[i] && v[i] < v2[i]));
    }
    assert!(is_similar(&mesh, &original, 0.014, 0.75));
    assert!(is_similar(&original, &mesh, 0.014, 0.75));
}

#[test]
fn simplify_mesh_by_quadric_edge_collapse_to_5_percent() {
    let original = crate::obj::load(data("frog_legs.obj")).expect("frog legs");
    let wanted = (original.triangles.len() as f64 * 0.05) as u32;
    let mut mesh = original.clone();
    run(&mut mesh, wanted, Some(f32::MAX));
    assert!(mesh.triangles.len() <= wanted as usize);
    assert!((original.signed_volume() - mesh.signed_volume()).abs() < 33.0);
    assert!(is_similar(&original, &mesh, 0.043, 0.32));
    assert!(is_similar(&mesh, &original, 0.043, 0.32));
}

#[test]
fn simplify_trouble_case() {
    let mut mesh = crate::obj::load(data("simplification.obj")).expect("trouble case");
    run(&mut mesh, 0, Some(f32::MAX));
    assert!(
        mesh.triangles
            .iter()
            .all(|t| t[0] != t[1] && t[0] != t[2] && t[1] != t[2])
    );
}

#[test]
fn simplified_cube_should_not_be_empty() {
    let mut mesh = its_make_cube(1.0, 2.0, 3.0);
    run(&mut mesh, 0, Some(f32::MAX));
    assert!(!mesh.triangles.is_empty());
}

#[test]
fn nothing_to_do_leaves_the_mesh() {
    let mut mesh = its_make_cube(1.0, 2.0, 3.0);
    let stats = run(&mut mesh, 12, None);
    assert_eq!(stats.triangles_after, 12);
    let stats = run(&mut mesh, 0, Some(0.0));
    assert_eq!(stats.triangles_after, 12);
    assert_eq!(mesh.vertices, its_make_cube(1.0, 2.0, 3.0).vertices);
}

#[test]
fn a_cancel_leaves_the_mesh_as_it_was() {
    let original = crate::obj::load(data("frog_legs.obj")).expect("frog legs");
    let mut mesh = original.clone();
    let asked = std::cell::Cell::new(0);
    let result = quadric_edge_collapse(&mut mesh, 100, None, &mut |_| {}, &|| {
        asked.set(asked.get() + 1);
        asked.get() > 50
    });
    assert_eq!(result, Err(Cancelled));
    assert_eq!(mesh.triangles, original.triangles);
}

#[test]
fn progress_runs_up_to_a_hundred() {
    let mut mesh = crate::obj::load(data("frog_legs.obj")).expect("frog legs");
    let mut seen = Vec::new();
    run_with(&mut mesh, &mut seen);
    assert!(seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert_eq!(seen.first(), Some(&0));
    assert!(*seen.last().expect("some") >= 99);
}

fn run_with(mesh: &mut Mesh, seen: &mut Vec<i32>) {
    let count = mesh.triangles.len() as u32 / 10;
    quadric_edge_collapse(mesh, count, None, &mut |p| seen.push(p), &|| false).expect("runs");
}

/// What the reference's own code (QuadricEdgeCollapse.cpp at the pinned
/// commit, built on its own with clang -O2) gave for these meshes and
/// parameters (`-O3`, clang's default contraction, the bundled Eigen), as
/// the corner and triangle counts, the last error's bits and FNV-1a over the
/// raw mesh. The same input is fed to both (the mesh as read here). 3DBenchy
/// and the Stanford Bunny gave the same bits too (the README's "Simplify").
/// File, triangles wanted, maximum error, corners and triangles left, the
/// last error's bits, FNV-1a.
type Case = (&'static str, u32, Option<f32>, usize, usize, u32, u64);

const REFERENCE: [Case; 5] = [
    (
        "frog_legs.obj",
        916,
        Some(f32::MAX),
        466,
        916,
        1067155820,
        12365737324000576520,
    ),
    (
        "frog_legs.obj",
        0,
        Some(0.1),
        1047,
        2078,
        1036823869,
        3198539524067338879,
    ),
    (
        "frog_legs.obj",
        0,
        Some(0.001),
        4753,
        9488,
        981665272,
        5342045250724655551,
    ),
    (
        "frog_legs.obj",
        0,
        Some(1.0),
        499,
        982,
        1065270574,
        7726250408328201246,
    ),
    (
        "simplification.obj",
        0,
        Some(f32::MAX),
        8,
        10,
        974964132,
        4634914048849393834,
    ),
];

#[test]
fn the_reference_gives_the_same_mesh() {
    for (file, count, max_error, vertices, triangles, last, hash) in REFERENCE {
        let mut mesh = crate::obj::load(data(file)).expect("reads");
        let stats = run(&mut mesh, count, max_error);
        assert_eq!(
            (mesh.vertices.len(), mesh.triangles.len()),
            (vertices, triangles),
            "{file} {count} {max_error:?}"
        );
        assert_eq!(
            stats.last_error.to_bits(),
            last,
            "{file} {count} {max_error:?}"
        );
        assert_eq!(
            raw::fnv(&raw::to_bytes(&mesh)),
            hash,
            "{file} {count} {max_error:?}"
        );
    }
}

/// Not a test: with EDGE_QEC_IN, EDGE_QEC_OUT, EDGE_QEC_COUNT and
/// EDGE_QEC_ERROR (a number or "inf") set, collapses a mesh (STL, OBJ or
/// raw) and writes it raw, for comparing with the reference's code.
#[test]
#[ignore]
fn collapse_a_file() {
    let input = std::env::var("EDGE_QEC_IN").expect("EDGE_QEC_IN");
    let mut mesh = raw::load_any(Path::new(&input));
    if let Ok(dump) = std::env::var("EDGE_QEC_DUMP") {
        std::fs::write(dump, raw::to_bytes(&mesh)).expect("writes");
    }
    let count: u32 = std::env::var("EDGE_QEC_COUNT")
        .expect("count")
        .parse()
        .expect("number");
    let error = std::env::var("EDGE_QEC_ERROR").expect("error");
    let error = if error == "inf" {
        f32::MAX
    } else {
        error.parse().expect("number")
    };
    let t = std::time::Instant::now();
    let stats = run(&mut mesh, count, Some(error));
    let took = t.elapsed();
    let out = std::env::var("EDGE_QEC_OUT").expect("EDGE_QEC_OUT");
    let bytes = raw::to_bytes(&mesh);
    std::fs::write(out, &bytes).expect("writes");
    println!(
        "{} {} {:.9e} bits {} fnv {} in {:?}",
        mesh.vertices.len(),
        mesh.triangles.len(),
        stats.last_error,
        stats.last_error.to_bits(),
        raw::fnv(&bytes),
        took
    );
}
