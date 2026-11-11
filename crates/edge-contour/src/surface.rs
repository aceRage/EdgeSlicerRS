//! The model's surface, which lines are contoured to: its triangles in a
//! tree, and a ray cast straight up from a point to where it leaves them.

use edge_mesh::Mesh;
use edge_visibility::{Bvh, Triangles};

use crate::sample::Hit;

/// The triangles of every part printed, in one tree.
#[derive(Clone, Debug, Default)]
pub struct Surface {
    triangles: Triangles,
    tree: Bvh,
}

impl Surface {
    /// The surface of `meshes`, as they stand on the bed.
    ///
    /// The reference casts each object's lines at that object's own
    /// mesh (PrintObject.cpp `contour_z`, the object's `raw_mesh`: its
    /// parts, not what is cut out of them nor modifiers). Here
    /// everything on the plate is sliced as one, and its lines are cast
    /// at every part on it: a ray up from one object meets another only
    /// where that one stands over it, within a layer of the line.
    pub fn new<'a>(meshes: impl IntoIterator<Item = &'a Mesh>) -> Self {
        let mut triangles = Triangles::default();
        for mesh in meshes {
            let from = triangles.vertices.len() as u32;
            triangles.vertices.extend_from_slice(&mesh.vertices);
            triangles.indices.extend(
                mesh.triangles
                    .iter()
                    .map(|corners| corners.map(|corner| corner + from)),
            );
        }
        let tree = Bvh::new(&triangles);
        Self { triangles, tree }
    }

    /// Where a ray straight up from `(x, y, z)` first leaves or meets the
    /// surface, if it is within `reach` of it, all in mm
    /// (SLA/IndexedMesh.cpp `query_ray_hit`, cast up only: casting both
    /// ways and taking the nearer could take the bottom for the top,
    /// upstream #13510). The reference casts each ray as far as it
    /// goes; one further than a point can be moved by counts for
    /// nothing, and here it is not looked for.
    pub fn up(&self, x: f64, y: f64, z: f64, reach: f64) -> Option<Hit> {
        let hit = self
            .tree
            .first_hit_within(&self.triangles, [x, y, z], [0.0, 0.0, 1.0], reach)?;
        let [a, b, c] = self.triangles.indices[hit.triangle as usize]
            .map(|corner| self.triangles.vertices[corner as usize].map(f64::from));
        let (u, v) = (
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
            [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
        );
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        Some(Hit {
            distance: f64::from(hit.t),
            normal,
        })
    }
}

#[cfg(test)]
mod tests {
    use edge_mesh::primitives;

    use super::*;

    #[test]
    fn a_ray_up_from_inside_a_block_leaves_by_its_top() {
        let block = Mesh::from_triangle_soup(primitives::cuboid([0.0; 3], [10.0, 10.0, 2.0]));
        let surface = Surface::new([&block]);
        let hit = surface.up(5.0, 5.0, 1.2, f64::INFINITY).unwrap();
        assert!((hit.distance - 0.8).abs() < 1e-6);
        assert!(hit.normal[2] > 0.0 && hit.normal[0] == 0.0 && hit.normal[1] == 0.0);
        assert_eq!(surface.up(5.0, 5.0, 1.2, 0.7), None);
        assert_eq!(surface.up(15.0, 5.0, 1.2, f64::INFINITY), None);
        assert_eq!(surface.up(5.0, 5.0, 2.5, f64::INFINITY), None);
    }
}
