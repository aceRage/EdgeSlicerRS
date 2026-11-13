use super::*;
use edge_mesh::{Mesh, hull::convex_hull, primitives::cuboid};
use std::f32::consts::FRAC_1_SQRT_2;

/// A mesh and its hull, turned by `turn` (a matrix applied to a column).
struct Solid {
    mesh: Mesh,
    hull: Mesh,
}

impl Solid {
    fn of(boxes: &[([f32; 3], [f32; 3])], turn: &Mat3) -> Self {
        let mesh = Mesh::from_triangle_soup(boxes.iter().flat_map(|(min, max)| {
            cuboid(*min, *max).into_iter().map(|t| {
                t.map(|p| {
                    let p = p.map(f64::from);
                    std::array::from_fn(|r| {
                        (turn[r][0] * p[0] + turn[r][1] * p[1] + turn[r][2] * p[2]) as f32
                    })
                })
            })
        }));
        let points: Vec<[f64; 3]> = mesh.vertices.iter().map(|v| v.map(f64::from)).collect();
        let hull = convex_hull(&points).unwrap();
        let hull = Mesh {
            vertices: hull.points.iter().map(|p| p.map(|v| v as f32)).collect(),
            triangles: hull.triangles,
        };
        Self { mesh, hull }
    }

    fn orient(&self, params: &Params) -> Orientation {
        orient(
            Faces {
                vertices: &self.mesh.vertices,
                triangles: &self.mesh.triangles,
            },
            Faces {
                vertices: &self.hull.vertices,
                triangles: &self.hull.triangles,
            },
            params,
        )
        .unwrap()
    }

    /// Its height once turned by `rotation`.
    fn height(&self, rotation: &Mat3) -> f64 {
        let z: Vec<f64> = self
            .mesh
            .vertices
            .iter()
            .map(|v| (0..3).map(|c| rotation[2][c] * v[c] as f64).sum())
            .collect();
        z.iter().copied().fold(f64::MIN, f64::max) - z.iter().copied().fold(f64::MAX, f64::min)
    }
}

const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

fn about(axis: usize, degrees: f64) -> Mat3 {
    let (s, c) = degrees.to_radians().sin_cos();
    let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
    let mut m = IDENTITY;
    m[i][i] = c;
    m[i][j] = -s;
    m[j][i] = s;
    m[j][j] = c;
    m
}

fn times(a: &Mat3, b: &Mat3) -> Mat3 {
    std::array::from_fn(|r| std::array::from_fn(|c| (0..3).map(|k| a[r][k] * b[k][c]).sum()))
}

fn close(a: &Mat3, b: &Mat3, tol: f64) -> bool {
    (0..3).all(|r| (0..3).all(|c| (a[r][c] - b[r][c]).abs() <= tol))
}

#[test]
fn a_tilted_box_is_laid_flat_on_its_largest_face() {
    let turn = times(&about(0, 27.0), &about(1, -41.0));
    let solid = Solid::of(&[([0.0; 3], [30.0, 20.0, 10.0])], &turn);
    for params in [Params::application(30.0), Params::command_line()] {
        let found = solid.orient(&params);
        assert!(
            (solid.height(&found.rotation) - 10.0).abs() < 1e-3,
            "{}",
            solid.height(&found.rotation)
        );
    }
}

#[test]
fn an_l_standing_up_is_laid_on_its_side() {
    // An L bracket's profile in XY, 10 mm thick, stood up on the end of
    // its long leg: laid back down it has no overhang at all.
    let l = [
        ([0.0, 0.0, 0.0], [60.0, 10.0, 10.0]),
        ([0.0, 10.0, 0.0], [10.0, 50.0, 10.0]),
    ];
    let stood = Solid::of(&l, &about(1, 90.0));
    let found = stood.orient(&Params::application(30.0));
    assert!((stood.height(&found.rotation) - 10.0).abs() < 1e-3);
    // Lying as it should, it stays.
    let lying = Solid::of(&l, &IDENTITY);
    assert_eq!(lying.orient(&Params::application(30.0)).rotation, IDENTITY);
}

#[test]
fn a_t_upside_down_is_turned_onto_its_bar() {
    // A T: a wide bar on a narrow post. Standing on the post it hangs the
    // bar's underside out; turned over it stands on the bar.
    let t = [
        ([20.0, 0.0, 0.0], [30.0, 10.0, 40.0]),
        ([0.0, 0.0, 40.0], [50.0, 10.0, 50.0]),
    ];
    let solid = Solid::of(&t, &IDENTITY);
    let found = solid.orient(&Params::application(30.0));
    let up = found.up;
    // Lying on its side (the T's profile down) or on its bar: either way
    // nothing hangs out, and both are better than the post.
    assert!(up[2] < -0.999 || up[1].abs() > 0.999, "{up:?}");
    let as_it_is = found.tried[0].costs.unprintability;
    let best = found
        .tried
        .iter()
        .map(|t| t.costs.unprintability)
        .fold(f32::MAX, f32::min);
    assert!(best < as_it_is);
}

#[test]
fn a_cube_on_the_bed_stays_as_it_is() {
    let solid = Solid::of(&[([0.0; 3], [20.0; 3])], &IDENTITY);
    for params in [Params::application(30.0), Params::command_line()] {
        let found = solid.orient(&params);
        assert_eq!(found.up, [0.0, 0.0, 1.0]);
        assert_eq!(found.rotation, IDENTITY);
    }
}

#[test]
fn the_same_at_any_thread_count() {
    let turn = times(&about(2, 13.0), &about(0, 71.0));
    let solid = Solid::of(
        &[
            ([0.0, 0.0, 0.0], [60.0, 10.0, 10.0]),
            ([0.0, 10.0, 0.0], [10.0, 50.0, 10.0]),
        ],
        &turn,
    );
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| solid.orient(&Params::application(30.0)))
    };
    let one = run(1);
    assert_eq!(one, run(4));
    assert_eq!(one, run(6));
}

#[test]
fn the_directions_tried_are_found_and_merged_as_the_reference_does() {
    // A 30 × 20 × 10 box: its own direction, its six faces (from the mesh,
    // then the hull's, which repeat them), and the 18 fixed ones less
    // those already there.
    let solid = Solid::of(&[([0.0; 3], [30.0, 20.0, 10.0])], &IDENTITY);
    let found = solid.orient(&Params::application(30.0));
    let ups: Vec<[f32; 3]> = found.tried.iter().map(|t| t.up).collect();
    assert_eq!(ups[0], [0.0, 0.0, 1.0]);
    // The mesh's faces by area: the 30 × 20 one on top (the bottom one
    // repeats the box's own direction), then the four sides.
    assert_eq!(ups[1], [0.0, 0.0, -1.0]);
    assert!(ups[2][1].abs() == 1.0 && ups[3][1].abs() == 1.0);
    assert!(ups[4][0].abs() == 1.0 && ups[5][0].abs() == 1.0);
    // The hull's faces are the same six; of the fixed 18, the six along
    // the axes were there already.
    assert_eq!(ups.len(), 1 + 5 + 12);
}

#[test]
fn the_costs_are_the_reference_formula() {
    // A 20 mm cube as it stands, by the command line's weights: 400 mm²
    // on the bed, all of it below half the first layer, so the bottom is
    // 400 · 0.5 + 400; its hull's two bottom triangles 400; nothing hangs
    // out, no face is nearly flat.
    let solid = Solid::of(&[([0.0; 3], [20.0; 3])], &IDENTITY);
    let params = Params::command_line();
    let found = solid.orient(&params);
    let costs = found.tried[0].costs;
    assert_eq!(costs.bottom, 600.0);
    assert_eq!(costs.bottom_hull, 400.0);
    assert_eq!(costs.overhang, 0.0);
    assert_eq!(costs.area_laf, 0.0);
    assert_eq!(costs.contour, 4.0 * 600f32.sqrt());
    let p = params;
    // Each `a * b + c` fused, as the reference's build has it.
    let under = p.bottom_hull_f.mul_add(
        400.0,
        p.bottom_f
            .mul_add(600.0, p.contour_f.mul_add(costs.contour, p.tar_d)),
    );
    let expected = p.relative_f * p.tar_d / under;
    assert_eq!(costs.unprintability, expected);
    // Within a hair of the plain sum.
    let plain = p.relative_f * p.tar_d
        / (p.tar_d + p.contour_f * costs.contour + p.bottom_f * 600.0 + p.bottom_hull_f * 400.0);
    assert!((costs.unprintability - plain).abs() < 1e-6);
    // Upside down the top is on the bed: the same. On its side likewise.
    // Turned 45° about X it stands on an edge: under the least bottom, so
    // 100 more.
    let edge = found
        .tried
        .iter()
        .find(|t| t.up == [0.0, -FRAC_1_SQRT_2, FRAC_1_SQRT_2])
        .unwrap();
    assert!(edge.costs.bottom < params.bottom_min);
    assert!(edge.costs.unprintability > 100.0);
}

#[test]
// Eigen's numbers as it printed them.
#[allow(clippy::excessive_precision)]
fn the_turn_is_eigens() {
    // Eigen 3.3.7's `setFromTwoVectors(from, (0, 0, 1)).toRotationMatrix()`,
    // printed by a program built against the reference's own Eigen.
    let flip = rotation_from_two_vectors([0.0, 0.0, -1.0], [0.0, 0.0, 1.0]);
    assert_eq!(flip, [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, -1.0]]);
    let near = rotation_from_two_vectors([3e-7, -2e-7, -1.0], [0.0, 0.0, 1.0]);
    let eigen = [
        [
            -0.384_615_383_445_471_53,
            0.923_076_923_564_338_24,
            2.996_236_668_401_863_7e-7,
        ],
        [
            0.923_076_923_564_338_24,
            0.384_615_383_445_536_36,
            -1.997_491_115_010_405_1e-7,
        ],
        [
            -2.996_236_668_401_863_7e-7,
            1.997_491_115_010_405_1e-7,
            -0.999_999_999_999_935_16,
        ],
    ];
    assert!(close(&near, &eigen, 1e-12), "{near:?}");
    let tilted = rotation_from_two_vectors([0.6, 0.0, -0.8], [0.0, 0.0, 1.0]);
    let eigen = [
        [-0.800_000_000_000_000_71, 0.0, -0.599_999_999_999_999_98],
        [0.0, 1.0, 0.0],
        [0.599_999_999_999_999_98, 0.0, -0.800_000_000_000_000_71],
    ];
    assert!(close(&tilted, &eigen, 1e-15), "{tilted:?}");
    assert_eq!(
        rotation_from_two_vectors([0.0, 0.0, 1.0], [0.0, 0.0, 1.0]),
        IDENTITY
    );
}

#[test]
fn the_overhang_angle_sets_the_ascent() {
    assert!((ascent(60.0) + 0.5).abs() < 1e-7);
    assert!((ascent(30.0) + 0.866_025_4).abs() < 1e-6);
    // 0: nothing hangs out.
    assert_eq!(ascent(0.0), -1.0);
}
