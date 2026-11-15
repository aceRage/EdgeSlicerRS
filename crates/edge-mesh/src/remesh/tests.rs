//! The remesh on shapes whose outcome is known: closed and wound outward,
//! edges about a voxel long, volume kept, a flat base kept flat, sharp
//! edges kept sharp, and broken meshes (holes, triangles the wrong way
//! round, an inside-out shell) made whole.

use super::*;
use crate::{boolean::check, primitives::cuboid};

fn never() -> bool {
    false
}

fn cube(low: [f32; 3], high: [f32; 3]) -> Mesh {
    Mesh::from_triangle_soup(cuboid(low, high))
}

/// A sphere of radius `r` about (0, 0, r): 96 by 48 bands.
fn sphere(r: f64) -> Mesh {
    let (rings, sectors) = (48usize, 96usize);
    let point = |ring: usize, sector: usize| -> [f32; 3] {
        if ring == 0 || ring == rings {
            return [0.0, 0.0, if ring == 0 { 2.0 * r as f32 } else { 0.0 }];
        }
        let theta = std::f64::consts::PI * ring as f64 / rings as f64;
        let phi = 2.0 * std::f64::consts::PI * (sector % sectors) as f64 / sectors as f64;
        [
            (r * theta.sin() * phi.cos()) as f32,
            (r * theta.sin() * phi.sin()) as f32,
            (r + r * theta.cos()) as f32,
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

fn closed_and_outward(mesh: &Mesh) {
    let d = check::defects(mesh);
    assert_eq!(d.open_edges, 0, "{d:?}");
    assert_eq!(d.doubled_edges, 0, "{d:?}");
    assert!(mesh.signed_volume() > 0.0);
}

fn edge_lengths(mesh: &Mesh) -> Vec<f64> {
    let mut out = Vec::new();
    for t in &mesh.triangles {
        for e in 0..3 {
            let (a, b) = (
                mesh.vertices[t[e] as usize],
                mesh.vertices[t[(e + 1) % 3] as usize],
            );
            out.push(
                (0..3)
                    .map(|i| f64::from(a[i] - b[i]).powi(2))
                    .sum::<f64>()
                    .sqrt(),
            );
        }
    }
    out
}

#[test]
fn a_sphere_comes_back_closed_with_edges_about_a_voxel_and_its_volume() {
    let ball = sphere(10.0);
    let voxel = 0.25;
    let made = voxel_remesh(&ball, voxel, &never).unwrap().unwrap();
    closed_and_outward(&made);
    let (v0, v1) = (ball.signed_volume(), made.signed_volume());
    assert!((v1 - v0).abs() / v0 < 0.01, "{v0} -> {v1}");
    // Edges of a quad grid: a side about a voxel, a diagonal about 1.4.
    let mut lengths = edge_lengths(&made);
    lengths.sort_by(f64::total_cmp);
    let median = lengths[lengths.len() / 2];
    assert!(
        median > 0.6 * voxel && median < 1.5 * voxel,
        "median {median}"
    );
    assert!(lengths[lengths.len() * 99 / 100] < 2.5 * voxel);
    // About two triangles a voxel of surface (RemeshDialog's estimate).
    let area = 4.0 * std::f64::consts::PI * 100.0;
    let estimate = 2.0 * area / (voxel * voxel);
    let n = made.triangles.len() as f64;
    assert!(
        n > 0.6 * estimate && n < 1.6 * estimate,
        "{n} of about {estimate}"
    );
    // Every corner on the sphere within a voxel.
    for v in &made.vertices {
        let r =
            (f64::from(v[0]).powi(2) + f64::from(v[1]).powi(2) + (f64::from(v[2]) - 10.0).powi(2))
                .sqrt();
        assert!((r - 10.0).abs() < voxel, "{r}");
    }
}

#[test]
fn the_same_mesh_gives_the_same_result() {
    let ball = sphere(5.0);
    let a = voxel_remesh(&ball, 0.2, &never).unwrap().unwrap();
    let b = voxel_remesh(&ball, 0.2, &never).unwrap().unwrap();
    assert_eq!(a.vertices, b.vertices);
    assert_eq!(a.triangles, b.triangles);
}

#[test]
fn a_mesh_with_a_hole_and_flipped_triangles_comes_back_whole() {
    let mut ball = sphere(8.0);
    // A hole of a few triangles, and some triangles turned round.
    ball.triangles.drain(1000..1006);
    for t in ball.triangles.iter_mut().skip(2000).take(40) {
        t.swap(1, 2);
    }
    assert!(!check::is_closed(&ball));
    let made = voxel_remesh(&ball, 0.25, &never).unwrap().unwrap();
    closed_and_outward(&made);
    let v0 = sphere(8.0).signed_volume();
    assert!((made.signed_volume() - v0).abs() / v0 < 0.02);
}

#[test]
fn overlapping_shells_are_joined_and_an_inside_out_one_is_left_out() {
    let mut two = cube([0.0; 3], [10.0; 3]);
    two.append(&cube([5.0; 3], [15.0; 3]), &Transform::IDENTITY);
    let made = voxel_remesh(&two, 0.25, &never).unwrap().unwrap();
    closed_and_outward(&made);
    let joined = 2.0 * 1000.0 - 125.0;
    assert!(
        (made.signed_volume() - joined).abs() / joined < 0.02,
        "{}",
        made.signed_volume()
    );

    // An inside-out cube beside a sound one: only the sound one is kept
    // (`mesh_to_grid` leaves out a shell of negative volume).
    let mut with = cube([0.0; 3], [10.0; 3]);
    let mut inverted = cube([20.0, 0.0, 0.0], [30.0, 10.0, 10.0]);
    for t in &mut inverted.triangles {
        t.swap(1, 2);
    }
    with.append(&inverted, &Transform::IDENTITY);
    let parts: Vec<f64> = crate::split::pieces(&with)
        .iter()
        .map(Mesh::signed_volume)
        .collect();
    assert_eq!(parts, [1000.0, -1000.0]);
    let made = voxel_remesh(&with, 0.25, &never).unwrap().unwrap();
    let (_, high) = made.bounds().unwrap();
    assert!(high[0] < 11.0, "{high:?} {parts:?}");
    assert!((made.signed_volume() - 1000.0).abs() < 20.0);
}

#[test]
fn a_cube_keeps_its_base_flat_and_its_edges_sharp() {
    let block = cube([0.0; 3], [20.0, 20.0, 10.0]);
    let (made, report) = remesh_with_options(
        &block,
        &Options {
            voxel_size: 0.5,
            ..Options::default()
        },
        [0.0, 0.0, -1.0],
        &never,
    )
    .unwrap()
    .unwrap();
    assert!(report.kept_bottom_flat, "{report:?}");
    assert!(report.sharp_snapped > 0);
    closed_and_outward(&made);
    // The base is the cube's own: at 0, the whole 20 × 20.
    let (low, high) = made.bounds().unwrap();
    assert_eq!(low[2], 0.0);
    let (area, _) = bottom_area(&made, [0.0, 0.0, -1.0], 1e-4);
    assert!((area - 400.0).abs() < 1e-2, "{area}");
    // The edges were pulled back onto the cube's: its box is the cube's.
    for i in 0..2 {
        assert!(
            low[i].abs() < 1e-4 && (high[i] - 20.0).abs() < 1e-4,
            "{low:?} {high:?}"
        );
    }
    assert!((high[2] - 10.0).abs() < 1e-4);
    assert!(
        (made.signed_volume() - 4000.0).abs() / 4000.0 < 0.01,
        "{}",
        made.signed_volume()
    );
}

#[test]
fn a_base_too_small_or_a_round_bottom_is_remeshed_whole() {
    // The sphere's lowest ring is 0.02 mm over its pole, past a twentieth
    // of a voxel: no flat base.
    let ball = sphere(10.0);
    let (made, report) = remesh_with_options(
        &ball,
        &Options {
            voxel_size: 0.25,
            ..Options::default()
        },
        [0.0, 0.0, -1.0],
        &never,
    )
    .unwrap()
    .unwrap();
    assert!(report.fell_back && !report.kept_bottom_flat, "{report:?}");
    closed_and_outward(&made);
}

#[test]
fn the_voxel_size_is_the_references_auto_one_and_clamped() {
    assert_eq!(auto_voxel_size(&Mesh::default()), 0.1);
    // A 20 mm cube: its diagonal over 300, 0.115.
    let v = auto_voxel_size(&cube([0.0; 3], [20.0; 3]));
    assert!((v - 20.0 * 3f64.sqrt() / 300.0).abs() < 1e-9);
    assert_eq!(auto_voxel_size(&cube([0.0; 3], [1.0; 3])), 0.05);
    assert_eq!(auto_voxel_size(&cube([0.0; 3], [200.0; 3])), 0.3);
    let (_, report) = remesh_with_options(
        &cube([0.0; 3], [2.0; 3]),
        &Options {
            voxel_size: 0.001,
            keep_bottom_flat: false,
            ..Options::default()
        },
        [0.0, 0.0, -1.0],
        &never,
    )
    .unwrap()
    .unwrap();
    assert_eq!(report.voxel_size, VOXEL_MIN);
}

#[test]
fn sharp_edges_are_found_as_the_reference_finds_them() {
    // A cube: its twelve edges and none of its six faces' diagonals.
    let block = cube([0.0; 3], [1.0; 3]);
    assert_eq!(detect_sharp_edges(&block, 45.0).len(), 12);
    // An open edge is always sharp.
    let mut open = block.clone();
    open.triangles.pop();
    assert_eq!(detect_sharp_edges(&open, 45.0).len(), 13);
}

#[test]
fn a_bed_below_another_way_is_turned_to() {
    // The bed along -x: the base is the face at x = 0.
    let block = cube([0.0; 3], [10.0, 20.0, 20.0]);
    let (made, report) = remesh_with_options(
        &block,
        &Options {
            voxel_size: 0.5,
            preserve_sharp_edges: false,
            ..Options::default()
        },
        [-1.0, 0.0, 0.0],
        &never,
    )
    .unwrap()
    .unwrap();
    assert!(report.kept_bottom_flat, "{report:?}");
    let (area, _) = bottom_area(&made, [-1.0, 0.0, 0.0], 1e-4);
    assert!((area - 400.0).abs() < 1e-2, "{area}");
}

#[test]
fn a_cancel_stops_it() {
    let ball = sphere(10.0);
    assert!(matches!(voxel_remesh(&ball, 0.1, &|| true), Err(Cancelled)));
}

/// Not a test: with EDGE_REMESH_IN (STL, OBJ or raw) and EDGE_REMESH_VOXEL
/// (mm, 0 for the auto size) set, times a "Repair/Remesh" with the
/// dialog's defaults and says what came of it.
#[test]
#[ignore]
fn remesh_a_file() {
    let input = std::env::var("EDGE_REMESH_IN").expect("EDGE_REMESH_IN");
    let mesh = crate::raw::load_any(std::path::Path::new(&input));
    let voxel: f64 = std::env::var("EDGE_REMESH_VOXEL")
        .expect("voxel")
        .parse()
        .expect("number");
    let t = std::time::Instant::now();
    let (made, report) = remesh_with_options(
        &mesh,
        &Options {
            voxel_size: voxel,
            ..Options::default()
        },
        [0.0, 0.0, -1.0],
        &never,
    )
    .unwrap()
    .unwrap();
    let took = t.elapsed();
    let d = check::defects(&made);
    println!(
        "{report:?} in {took:?}; {d:?}; volume {} -> {}",
        mesh.signed_volume(),
        made.signed_volume()
    );
}

#[test]
fn a_coarse_sphere_with_a_hole_comes_back_with_no_open_edge() {
    let (rings, sectors) = (24usize, 48usize);
    let point = |ring: usize, sector: usize| -> [f32; 3] {
        if ring == 0 || ring == rings {
            return [0.0, 0.0, if ring == 0 { 16.0 } else { 0.0 }];
        }
        let theta = std::f64::consts::PI * ring as f64 / rings as f64;
        let phi = 2.0 * std::f64::consts::PI * (sector % sectors) as f64 / sectors as f64;
        [
            (8.0 * theta.sin() * phi.cos()) as f32,
            (8.0 * theta.sin() * phi.sin()) as f32,
            (8.0 + 8.0 * theta.cos()) as f32,
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
    let mut mesh = Mesh::from_triangle_soup(soup);
    mesh.triangles.drain(500..503);
    let voxel = 16.0 * 3f64.sqrt() / 300.0;
    let plain = voxel_remesh(&mesh, voxel, &never).unwrap().unwrap();
    let open = |m: &Mesh| {
        crate::split::neighbours(m)
            .iter()
            .flatten()
            .filter(|n| n.is_none())
            .count()
    };
    assert_eq!(
        (check::defects(&plain).open_edges, open(&plain)),
        (0, 0),
        "{:?}",
        check::defects(&plain)
    );
    let (made, _) = remesh_with_options(
        &mesh,
        &Options {
            voxel_size: voxel,
            ..Options::default()
        },
        [0.0, 0.0, -1.0],
        &never,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        (check::defects(&made).open_edges, open(&made)),
        (0, 0),
        "{:?}",
        check::defects(&made)
    );
}
