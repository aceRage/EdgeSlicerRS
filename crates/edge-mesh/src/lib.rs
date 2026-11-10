//! Indexed triangle meshes and the file formats that produce them.

pub mod primitives;
pub mod stl;

use rustc_hash::FxHashMap;

/// Three corners in millimetres, wound counter-clockwise seen from outside.
pub type Triangle = [[f32; 3]; 3];

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

impl Mesh {
    /// Welds bit-identical corners into shared vertices, which is what lets
    /// the slicer chain contours by edge identity instead of by distance.
    /// Triangles that collapse onto an edge or a point are dropped.
    pub fn from_triangle_soup(soup: impl IntoIterator<Item = Triangle>) -> Self {
        let mut index: FxHashMap<[u32; 3], u32> = FxHashMap::default();
        let mut vertices: Vec<[f32; 3]> = Vec::new();
        let mut triangles = Vec::new();
        for corners in soup {
            let ids = corners.map(|p| {
                // Adding 0.0 folds -0.0 into +0.0 so both weld to one vertex.
                let key = p.map(|c| (c + 0.0).to_bits());
                *index.entry(key).or_insert_with(|| {
                    vertices.push(p);
                    u32::try_from(vertices.len() - 1).expect("mesh exceeds u32 vertex indices")
                })
            });
            if ids[0] != ids[1] && ids[1] != ids[2] && ids[2] != ids[0] {
                triangles.push(ids);
            }
        }
        Self {
            vertices,
            triangles,
        }
    }

    pub fn triangle(&self, index: usize) -> Triangle {
        self.triangles[index].map(|v| self.vertices[v as usize])
    }

    /// Axis-aligned bounds as `(min, max)`, or `None` for an empty mesh.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        let first = *self.vertices.first()?;
        Some(
            self.vertices
                .iter()
                .fold((first, first), |(mut lo, mut hi), v| {
                    for axis in 0..3 {
                        lo[axis] = lo[axis].min(v[axis]);
                        hi[axis] = hi[axis].max(v[axis]);
                    }
                    (lo, hi)
                }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn welds_shared_corners() {
        let mesh = Mesh::from_triangle_soup(primitives::cuboid([0.0; 3], [1.0, 2.0, 3.0]));
        assert_eq!(mesh.vertices.len(), 8);
        assert_eq!(mesh.triangles.len(), 12);
        assert_eq!(mesh.bounds(), Some(([0.0; 3], [1.0, 2.0, 3.0])));
    }

    #[test]
    fn drops_collapsed_triangles_and_folds_negative_zero() {
        let a = [0.0, 0.0, 0.0];
        let mesh = Mesh::from_triangle_soup([
            [a, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[-0.0, 0.0, 0.0], a, [1.0, 0.0, 0.0]],
        ]);
        assert_eq!(mesh.vertices.len(), 3);
        assert_eq!(mesh.triangles.len(), 1);
    }
}
