//! A mesh placed as the reference hands it to its slicer.
//!
//! The reference keeps each part's mesh in single precision about the
//! middle of its own box (Model.cpp `ModelVolume::center_geometry_after_creation`,
//! moved there as `add_volume` makes the part), and cuts every copy of an
//! object in the object's own frame, about the middle of the object's box
//! (PrintObject.cpp: `trafo_centered`), adding where the copy stands
//! (`PrintInstance::shift`) to the cut outlines in whole coordinate units.
//! Before cutting, it turns the part's map to that frame into single
//! precision, scaled to coordinate units across the bed, and moves each
//! corner by it in single precision (TriangleMeshSlicer.cpp
//! `transform_mesh_vertices_for_slicing`). So an outline the reference
//! cuts lies on the grid single precision gives a corner that far from
//! the middle of its object, which is finer than single precision on the
//! bed, and copies of one object are cut the same to the unit.

use crate::{Mesh, Transform};

/// One coordinate unit is a millionth of a millimetre (`SCALING_FACTOR`).
const UNITS_PER_MM: f64 = 1e6;

/// A mesh in the reference's slicing frame: across the bed (x, y) in
/// coordinate units from the middle of its object, up (z) in millimetres
/// from the bed, every corner in single precision; with where that middle
/// stands, in whole coordinate units from the plate's origin.
#[derive(Clone, Debug, Default)]
pub struct SlicingMesh {
    pub mesh: Mesh,
    pub shift: [i64; 2],
    /// Each corner of `mesh` in millimetres in the object's frame, moved
    /// in double precision: what the reference tells faces that lie in
    /// one plane by (TriangleMeshSlicer.cpp `coplanar_edges`, the part's
    /// map applied to the single-precision corner in double precision).
    pub exact: Vec<[f64; 3]>,
}

impl SlicingMesh {
    /// An empty mesh of a copy standing at `shift`.
    pub fn at(shift: [i64; 2]) -> Self {
        Self {
            mesh: Mesh::default(),
            shift,
            exact: Vec::new(),
        }
    }

    /// Adds `other`, a mesh of the same copy, as a shell of its own.
    pub fn extend(&mut self, other: &SlicingMesh) {
        assert_eq!(self.shift, other.shift, "meshes of one copy");
        let offset =
            u32::try_from(self.mesh.vertices.len()).expect("mesh exceeds u32 vertex indices");
        self.mesh.vertices.extend_from_slice(&other.mesh.vertices);
        self.exact.extend_from_slice(&other.exact);
        (self.mesh.triangles).extend(other.mesh.triangles.iter().map(|t| t.map(|v| v + offset)));
    }

    /// Adds `local`, a part's mesh in its own frame, moved by `to_object`
    /// (millimetres, the part's frame to its object's, the middle of the
    /// object's box at the origin across the bed) as the reference moves
    /// it for cutting: the map scaled to coordinate units across the bed
    /// and rounded to single precision, and each corner `x · c0 + y · c1 +
    /// z · c2 + t`, the first product rounded and the next two fused, as
    /// the pinned build's Eigen code does it (`fmul`, `fmla`, `fmla`,
    /// `fadd`). A map that mirrors swaps two corners of each triangle, as
    /// [`Mesh::append`] does.
    pub fn append(&mut self, local: &Mesh, to_object: &Transform) {
        // Column c of the map, as single-precision numbers: what x, y, z
        // and 1 are multiplied by for output coordinate c.
        let r = &to_object.rows;
        let scale = |c: usize| if c < 2 { UNITS_PER_MM } else { 1.0 };
        let m: [[f32; 4]; 3] =
            std::array::from_fn(|c| std::array::from_fn(|i| (r[i][c] * scale(c)) as f32));
        let offset =
            u32::try_from(self.mesh.vertices.len()).expect("mesh exceeds u32 vertex indices");
        self.mesh
            .vertices
            .extend(local.vertices.iter().map(|&[x, y, z]| {
                std::array::from_fn(|c| {
                    let [cx, cy, cz, t] = m[c];
                    t + cz.mul_add(z, cy.mul_add(y, cx * x))
                })
            }));
        (self.exact).extend(
            local
                .vertices
                .iter()
                .map(|v| to_object.apply(v.map(f64::from))),
        );
        let mirrored = to_object.is_mirrored();
        self.mesh
            .triangles
            .extend(local.triangles.iter().map(|&[a, b, c]| {
                let [a, b, c] = [a + offset, b + offset, c + offset];
                if mirrored { [a, c, b] } else { [a, b, c] }
            }));
    }
}

/// A part's mesh moved to the middle of its own box as the reference
/// does when it makes the part (Model.cpp
/// `ModelVolume::center_geometry_after_creation`): the middle taken in
/// double precision from the corners' least and greatest values, rounded
/// to single precision, and taken off each corner in single precision.
/// With the middle in double precision, which the part's map then adds
/// back (`ModelVolume::translate(shift)`).
pub fn centred(mesh: &Mesh) -> (Mesh, [f64; 3]) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for v in &mesh.vertices {
        for c in 0..3 {
            lo[c] = lo[c].min(v[c]);
            hi[c] = hi[c].max(v[c]);
        }
    }
    if mesh.vertices.is_empty() {
        return (mesh.clone(), [0.0; 3]);
    }
    let middle: [f64; 3] = std::array::from_fn(|c| (f64::from(lo[c]) + f64::from(hi[c])) / 2.0);
    if middle == [0.0; 3] {
        return (mesh.clone(), middle);
    }
    let by = middle.map(|c| -(c as f32));
    let vertices = (mesh.vertices.iter())
        .map(|v| std::array::from_fn(|c| v[c] + by[c]))
        .collect();
    (
        Mesh {
            vertices,
            triangles: mesh.triangles.clone(),
        },
        middle,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned build, under lldb, on m2b_three: its first block runs
    /// from 155 to 168.3 mm (single precision), and the part's corners
    /// it cuts lie at -6.6499939 and 6.65000916 about the part's middle:
    /// the middle, 161.650001525..., rounded to single precision first.
    #[test]
    fn a_part_is_centred_as_the_reference_centres_it() {
        let block = Mesh {
            vertices: vec![[155.0, 150.0, 0.0], [168.3, 170.0, 6.0]],
            triangles: Vec::new(),
        };
        let (local, middle) = centred(&block);
        assert_eq!(middle, [(155.0 + f64::from(168.3f32)) / 2.0, 160.0, 3.0]);
        assert_eq!(local.vertices[0], [-6.649_994, -10.0, -3.0]);
        assert_eq!(local.vertices[1], [6.650_009, 10.0, 3.0]);
    }

    /// The same block's corners as cut there: its part's map takes it
    /// -13.349998474121094 mm across from the object's middle; in
    /// coordinate units the corners come out at -19999993 and -6699989.
    #[test]
    fn a_part_is_moved_for_cutting_in_single_precision() {
        let local = Mesh {
            vertices: vec![[-6.649_994, -10.0, -3.0], [6.650_009, 10.0, 3.0]],
            triangles: Vec::new(),
        };
        let mut cut = SlicingMesh::at([175_000_000, 160_000_000]);
        cut.append(
            &local,
            &Transform::translation([-13.349_998_474_121_094, 0.0, 3.0]),
        );
        assert_eq!(cut.mesh.vertices[1], [-6_699_989.0, 10_000_000.0, 6.0]);
        assert_eq!(cut.mesh.vertices[0][2], 0.0);
        assert_eq!(cut.mesh.vertices[0][0].round(), -19_999_992.0);
    }
}
