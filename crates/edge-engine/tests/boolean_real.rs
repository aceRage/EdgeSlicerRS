//! The solid boolean on real meshes, for correctness at scale and for
//! time: the 3DBenchy less a box, less a cylinder through its cabin and
//! intersected with a sphere; a 1.4-million-triangle figurine less a box.
//! Each result is checked (closed, consistently wound, no triangle of
//! zero area, the volumes adding up) and timed.
//!
//! The meshes are not in the repository. Point the variables at projects
//! and run in release:
//!
//! ```text
//! EDGE_BOOLEAN_BENCHY=<benchy.3mf> EDGE_BOOLEAN_FIGURINE=<figurine.3mf> \
//!   cargo test --release -p edge-engine --test boolean_real -- --ignored --nocapture
//! ```
//!
//! `EDGE_BOOLEAN_OUT=<folder>` also writes each result there as an STL.

use std::time::Instant;

use edge_mesh::Mesh;
use edge_mesh::boolean::{Op, Solid, check};

fn load(variable: &str) -> Option<Mesh> {
    let path = std::env::var(variable).ok()?;
    let project = edge_3mf::Project::open(&path).expect("the project reads");
    Some(project.mesh(None))
}

fn cube(min: [f64; 3], max: [f64; 3]) -> Mesh {
    let f = |p: [f64; 3]| p.map(|c| c as f32);
    Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(f(min), f(max)))
}

/// A cylinder along y.
fn cylinder_y(x: f64, z: f64, r: f64, y0: f64, y1: f64, seg: usize) -> Mesh {
    let at = |j: usize, y: f64| {
        let a = std::f64::consts::TAU * (j % seg) as f64 / seg as f64;
        [(x + r * a.cos()) as f32, y as f32, (z + r * a.sin()) as f32]
    };
    let (front, back) = (
        [x as f32, y0 as f32, z as f32],
        [x as f32, y1 as f32, z as f32],
    );
    let mut soup = Vec::new();
    for j in 0..seg {
        let (a, b, c, d) = (at(j, y0), at(j + 1, y0), at(j + 1, y1), at(j, y1));
        soup.extend([[a, c, b], [a, d, c], [front, a, b], [back, c, d]]);
    }
    Mesh::from_triangle_soup(soup)
}

fn sphere(c: [f64; 3], r: f64, seg: usize) -> Mesh {
    let rings = seg / 2;
    let p = |i: usize, j: usize| -> [f32; 3] {
        let th = std::f64::consts::PI * i as f64 / rings as f64;
        let ph = std::f64::consts::TAU * (j % seg) as f64 / seg as f64;
        let s = if i == 0 || i == rings {
            0.0
        } else {
            r * th.sin()
        };
        [
            (c[0] + s * ph.cos()) as f32,
            (c[1] + s * ph.sin()) as f32,
            (c[2] - r * th.cos()) as f32,
        ]
    };
    let mut soup = Vec::new();
    for i in 0..rings {
        for j in 0..seg {
            let (a, b, cc, d) = (p(i, j), p(i, j + 1), p(i + 1, j + 1), p(i + 1, j));
            soup.extend([[a, b, cc], [a, cc, d]]);
        }
    }
    Mesh::from_triangle_soup(soup)
}

/// Runs `a op b`, checks it, and prints how long each step took.
fn run(name: &str, a: &Mesh, b: &Mesh, op: Op) -> Mesh {
    let t = Instant::now();
    let sa = Solid::new(a).expect("the mesh is a solid");
    let prepared_a = t.elapsed().as_secs_f64();
    let sb = Solid::new(b).expect("the tool is a solid");
    let t = Instant::now();
    let solid = sa.boolean(&sb, op).expect("the boolean");
    let booleaned = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let mesh = solid.mesh().expect("the result rounds");
    let rounded = t.elapsed().as_secs_f64();
    let defects = check::defects(&mesh);
    assert!(defects.is_clean(), "{name}: {defects:?}");
    // The volumes add up, against the intersection.
    let both = sa.boolean(&sb, Op::Intersection).unwrap().volume();
    let (va, vb) = (sa.volume(), sb.volume());
    let expected = match op {
        Op::Union => va + vb - both,
        Op::Difference => va - both,
        Op::Intersection => both,
    };
    let got = mesh.signed_volume();
    assert!(
        (got - expected).abs() < 1e-6 * (va + vb),
        "{name}: {got} mm³ against {expected}"
    );
    println!(
        "{name}: {} + {} triangles -> {}, {got:.3} mm³; the mesh made a solid in {prepared_a:.3} s \
         (crosses itself: {}), the boolean {booleaned:.3} s, rounding and checks {rounded:.3} s",
        a.triangles.len(),
        b.triangles.len(),
        mesh.triangles.len(),
        sa.is_self_intersecting(),
    );
    if let Ok(folder) = std::env::var("EDGE_BOOLEAN_OUT") {
        let path = std::path::Path::new(&folder).join(format!("{name}.stl"));
        std::fs::write(path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    }
    mesh
}

#[test]
#[ignore = "needs EDGE_BOOLEAN_BENCHY; run in release"]
fn benchy() {
    let Some(benchy) = load("EDGE_BOOLEAN_BENCHY") else {
        return;
    };
    let (lo, hi) = benchy.bounds().unwrap();
    let (lo, hi) = (lo.map(f64::from), hi.map(f64::from));
    let mid: [f64; 3] = std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0);
    // The bow quarter, cut away to the deck.
    let corner = cube(
        [mid[0], lo[1] - 1.0, lo[2] - 1.0],
        [hi[0] + 1.0, mid[1], mid[2]],
    );
    run("benchy_minus_box", &benchy, &corner, Op::Difference);
    // Through the cabin, across the hull.
    let through = cylinder_y(
        mid[0] - 2.0,
        lo[2] + 0.62 * (hi[2] - lo[2]),
        6.0,
        lo[1] - 5.0,
        hi[1] + 5.0,
        96,
    );
    run("benchy_minus_cylinder", &benchy, &through, Op::Difference);
    let ball = sphere(mid, 0.4 * (hi[0] - lo[0]), 96);
    run("benchy_and_sphere", &benchy, &ball, Op::Intersection);
}

#[test]
#[ignore = "needs EDGE_BOOLEAN_FIGURINE; run in release"]
fn figurine() {
    let Some(figurine) = load("EDGE_BOOLEAN_FIGURINE") else {
        return;
    };
    let (lo, hi) = figurine.bounds().unwrap();
    let (lo, hi) = (lo.map(f64::from), hi.map(f64::from));
    let mid: [f64; 3] = std::array::from_fn(|i| (lo[i] + hi[i]) / 2.0);
    let corner = cube(
        [mid[0], lo[1] - 1.0, lo[2] - 1.0],
        [hi[0] + 1.0, mid[1], mid[2]],
    );
    run("figurine_minus_box", &figurine, &corner, Op::Difference);
}
