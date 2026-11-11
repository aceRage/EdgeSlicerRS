//! Which edges of a mesh lie inside a flat face.
//!
//! A flat face is usually several triangles, and a cutting plane that
//! crosses an edge between two of them puts a point on the outline that
//! is no corner of the model. The reference leaves those points out
//! (TriangleMeshSlicer.cpp `coplanar_edges` and
//! `chain_lines_by_triangle_connectivity`: "Skip only junctions
//! introduced by triangulating one planar face"); [`flat_edges`] is its
//! test of an edge.

use edge_mesh::Mesh;
use rustc_hash::{FxHashMap, FxHashSet};

/// How far, in mm, a triangle's far corner may be from its neighbour's
/// plane for the two to count as one face: one coordinate unit
/// (`plane_distance_tolerance = SCALING_FACTOR`).
const PLANE_DISTANCE_TOLERANCE: f64 = 0.000_001;

/// An edge's key, as the slicer keys the points it cuts on it.
pub(crate) fn edge_key(a: u32, b: u32) -> u64 {
    let (low, high) = if a < b { (a, b) } else { (b, a) };
    u64::from(low) << 32 | u64::from(high)
}

/// The edges whose two triangles lie in one plane, facing the same way.
/// An edge of more than two triangles goes by its first and its last, as
/// in the reference.
pub(crate) fn flat_edges(mesh: &Mesh) -> FxHashSet<u64> {
    let corner = |vertex: u32| mesh.vertices[vertex as usize].map(f64::from);
    let minus = |a: [f64; 3], b: [f64; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // A triangle's first corner and its unit normal; none for a
    // triangle of no area.
    let plane = |face: usize| -> Option<([f64; 3], [f64; 3])> {
        let [a, b, c] = mesh.triangles[face].map(corner);
        let (u, v) = (minus(b, a), minus(c, a));
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let length = dot(normal, normal).sqrt();
        (length > 0.0).then(|| (a, normal.map(|part| part / length)))
    };
    // The first triangle met on each edge, and its corner off the edge.
    let mut first: FxHashMap<u64, (usize, u32)> = FxHashMap::default();
    let mut flat = FxHashSet::default();
    for (face, ids) in mesh.triangles.iter().enumerate() {
        for side in 0..3 {
            let key = edge_key(ids[side], ids[(side + 1) % 3]);
            let opposite = ids[(side + 2) % 3];
            let Some(&(other, other_opposite)) = first.get(&key) else {
                first.insert(key, (face, opposite));
                continue;
            };
            let same = match (plane(other), plane(face)) {
                (Some((origin_1, normal_1)), Some((origin_2, normal_2))) => {
                    dot(normal_1, normal_2) > 0.0
                        && dot(normal_1, minus(corner(opposite), origin_1)).abs()
                            <= PLANE_DISTANCE_TOLERANCE
                        && dot(normal_2, minus(corner(other_opposite), origin_2)).abs()
                            <= PLANE_DISTANCE_TOLERANCE
                }
                _ => false,
            };
            if same {
                flat.insert(key);
            } else {
                flat.remove(&key);
            }
        }
    }
    flat
}

#[cfg(test)]
mod tests {
    use edge_mesh::primitives::cuboid;

    use super::*;

    #[test]
    fn the_diagonals_of_a_boxes_faces_are_flat_and_its_edges_are_not() {
        let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], [10.0, 20.0, 30.0]));
        let flat = flat_edges(&mesh);
        // Six faces, a diagonal each.
        assert_eq!(flat.len(), 6);
        // Each joins two corners that differ along two axes.
        for key in &flat {
            let (a, b) = ((key >> 32) as usize, (key & 0xffff_ffff) as usize);
            let differing = (0..3)
                .filter(|&axis| mesh.vertices[a][axis] != mesh.vertices[b][axis])
                .count();
            assert_eq!(differing, 2);
        }
    }

    #[test]
    fn a_box_is_cut_into_its_four_corners_without_the_crossings() {
        let mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], [10.0, 20.0, 30.0]));
        let planes = [0.1, 7.3, 15.0, 29.9];
        // The plain cut has a point on each face's diagonal as well.
        for layer in crate::slice(&mesh, &planes) {
            assert_eq!(layer.polygons.len(), 1);
            assert_eq!(layer.polygons[0].points.len(), 8);
        }
        for layer in crate::slice_without_flat_crossings(&mesh, &planes) {
            assert_eq!(layer.polygons.len(), 1);
            assert_eq!(layer.polygons[0].points.len(), 4);
            assert!((layer.polygons[0].signed_area_mm2() - 200.0).abs() < 1e-9);
            assert_eq!(layer.open_chains, 0);
        }
    }

    #[test]
    fn a_fold_between_two_triangles_is_no_flat_edge() {
        // Two triangles on an edge, the second's far corner a
        // hundredth of a millimetre out of the first's plane.
        let fold = |lift: f32| {
            Mesh::from_triangle_soup([
                [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
                [[10.0, 0.0, 0.0], [10.0, 10.0, lift], [0.0, 10.0, 0.0]],
            ])
        };
        assert_eq!(flat_edges(&fold(0.0)).len(), 1);
        assert!(flat_edges(&fold(0.01)).is_empty());
        // Nor two triangles in one plane that face opposite ways.
        let folded_back = Mesh::from_triangle_soup([
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 10.0, 0.0]],
            [[0.0, 10.0, 0.0], [10.0, 10.0, 0.0], [10.0, 0.0, 0.0]],
        ]);
        assert!(flat_edges(&folded_back).is_empty());
    }
}
