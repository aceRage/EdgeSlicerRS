//! Which edges of a mesh lie inside a flat face.
//!
//! A flat face is usually several triangles, and a cutting plane that
//! crosses an edge between two of them puts a point on the outline that
//! is no corner of the model. The reference leaves those points out
//! (TriangleMeshSlicer.cpp `coplanar_edges` and
//! `chain_lines_by_triangle_connectivity`: "Skip only junctions
//! introduced by triangulating one planar face"), and so does every
//! cut made here; [`flat_edges`] is its test of an edge.

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
    use edge_geom::{Point2, scaled};
    use edge_mesh::Triangle;
    use edge_mesh::primitives::cuboid;

    use super::*;

    /// A round post of so many sides standing on the bed, as a modeller
    /// writes one: each side two triangles, each end a fan about its
    /// middle. With it, its corners as the mesh has them.
    fn post(sides: usize, radius: f64, height: f32) -> (Vec<Triangle>, Vec<[f32; 2]>) {
        let (cx, cy) = (135.5, 136.0);
        let corners: Vec<[f32; 2]> = (0..sides)
            .map(|i| {
                let turn = std::f64::consts::TAU * i as f64 / sides as f64;
                [
                    (cx + radius * turn.cos()) as f32,
                    (cy + radius * turn.sin()) as f32,
                ]
            })
            .collect();
        let (middle, at) = ([cx as f32, cy as f32], |p: [f32; 2], z: f32| {
            [p[0], p[1], z]
        });
        let mut soup = Vec::new();
        for i in 0..sides {
            let (a, b) = (corners[i], corners[(i + 1) % sides]);
            soup.push([at(a, 0.0), at(b, 0.0), at(b, height)]);
            soup.push([at(a, 0.0), at(b, height), at(a, height)]);
            soup.push([at(middle, 0.0), at(b, 0.0), at(a, 0.0)]);
            soup.push([at(middle, height), at(a, height), at(b, height)]);
        }
        (soup, corners)
    }

    /// The contour's points, turned to start at the one nearest `first`.
    fn from_nearest(points: &[Point2], first: Point2) -> Vec<Point2> {
        let far = |p: &Point2| (p.x - first.x).abs() + (p.y - first.y).abs();
        let start = (0..points.len()).min_by_key(|&i| far(&points[i])).unwrap();
        let mut turned = points.to_vec();
        turned.rotate_left(start);
        turned
    }

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
        // Each plane crosses the diagonal of all four upright faces,
        // and none of those crossings is a point of the outline.
        for layer in crate::slice(&mesh, &planes) {
            assert_eq!(layer.polygons.len(), 1);
            assert_eq!(layer.polygons[0].points.len(), 4);
            assert!((layer.polygons[0].signed_area_mm2() - 200.0).abs() < 1e-9);
            assert_eq!(layer.open_chains, 0);
        }
    }

    #[test]
    fn a_round_post_is_cut_into_its_own_corners() {
        let (soup, corners) = post(64, 10.0, 10.0);
        let mesh = Mesh::from_triangle_soup(soup);
        let expected: Vec<Point2> = corners
            .iter()
            .map(|c| Point2::from_mm(c[0].into(), c[1].into()))
            .collect();
        // The last plane passes through the corners at the top.
        for layer in crate::slice(&mesh, &[0.1, 3.7, 9.9, 10.0]) {
            assert_eq!(layer.open_chains, 0, "z = {}", layer.z);
            assert_eq!(layer.polygons.len(), 1, "z = {}", layer.z);
            let points = from_nearest(&layer.polygons[0].points, expected[0]);
            assert_eq!(points, expected, "z = {}", layer.z);
        }
    }

    #[test]
    fn a_corner_the_plane_passes_through_is_kept_wherever_the_contour_starts() {
        // A contour starts at the first triangle the plane crosses, so
        // each triangle is put first in turn: among them the ones whose
        // first crossing is on a face's diagonal, at the corner itself
        // when the plane is level with the top.
        let soup = cuboid([0.0; 3], [10.0, 20.0, 30.0]);
        for first in 0..soup.len() {
            let mut turned = soup.clone();
            turned.rotate_left(first);
            let mesh = Mesh::from_triangle_soup(turned);
            for layer in crate::slice(&mesh, &[15.0, 30.0]) {
                assert_eq!(layer.open_chains, 0);
                assert_eq!(layer.polygons.len(), 1, "from triangle {first}");
                let ring = &layer.polygons[0];
                assert_eq!(ring.points.len(), 4, "from triangle {first}");
                assert!((ring.signed_area_mm2() - 200.0).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn closing_a_round_outline_gives_its_corners_back() {
        // Closed by the reference's default `slice_closing_radius`, the
        // post's outline is still its 64 corners. With the crossings on
        // the sides' diagonals left in, each a unit or two off its side,
        // closing left kinks hundredths of a millimetre from them.
        let (soup, corners) = post(64, 10.0, 10.0);
        let mesh = Mesh::from_triangle_soup(soup);
        let body = crate::Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        };
        let layers = crate::slice_regions(&[body], &[0.1, 3.7, 9.9], scaled(0.049)).unwrap();
        for layer in layers {
            assert_eq!(layer.regions.len(), 1, "z = {}", layer.z);
            let points = &layer.regions[0].contour.points;
            assert_eq!(points.len(), corners.len(), "z = {}", layer.z);
            let furthest = corners
                .iter()
                .map(|corner| {
                    let corner = Point2::from_mm(corner[0].into(), corner[1].into());
                    points
                        .iter()
                        .map(|p| (p.x - corner.x).abs().max((p.y - corner.y).abs()))
                        .min()
                        .unwrap()
                })
                .max()
                .unwrap();
            // Within what growing and shrinking rounds a point by:
            // a hundred-thousandth of a millimetre.
            assert!(furthest <= 10, "z = {}: {furthest} units off", layer.z);
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
