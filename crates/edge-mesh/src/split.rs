//! A mesh as the pieces it is made of: the shells that share no edge with
//! one another (libslic3r/MeshSplitImpl.hpp `its_split`,
//! `its_is_splittable`, `create_face_neighbors_index`; TriangleMesh.cpp
//! `TriangleMesh::split`).
//!
//! Two triangles are of one piece when they share an edge the other way
//! round, as two faces of a closed surface do; touching at a corner, or
//! along an edge both run the same way, does not join them. The pieces come
//! in the reference's order: each starts at the first triangle no earlier
//! piece holds, and its triangles and corners are taken in the order the
//! reference's walk takes them.

use crate::Mesh;

/// Each triangle's neighbour across each of its edges (edge `k` runs from
/// corner `k` to corner `k + 1`), as the reference pairs them: across an
/// edge, the first later triangle (by index) that runs it the other way
/// round and has that edge free. Worked out in one pass, in order, so a
/// mesh whose edges are shared by more than two triangles gives the same
/// pairs every time (the reference pairs them on several threads at once,
/// which can pair them otherwise from one run to the next).
pub fn neighbours(mesh: &Mesh) -> Vec<[Option<u32>; 3]> {
    let count = mesh.triangles.len();
    // The triangles round each corner, in order.
    let mut starts = vec![0u32; mesh.vertices.len() + 1];
    for t in &mesh.triangles {
        for &v in t {
            starts[v as usize + 1] += 1;
        }
    }
    for at in 1..starts.len() {
        starts[at] += starts[at - 1];
    }
    let mut filled = starts.clone();
    let mut round = vec![0u32; starts[mesh.vertices.len()] as usize];
    for (face, t) in mesh.triangles.iter().enumerate() {
        for &v in t {
            round[filled[v as usize] as usize] = face as u32;
            filled[v as usize] += 1;
        }
    }
    let mut out: Vec<[Option<u32>; 3]> = vec![[None; 3]; count];
    for face in 0..count {
        let t = mesh.triangles[face];
        for k in 0..3 {
            if out[face][k].is_some() {
                continue;
            }
            let (from, to) = (t[k], t[(k + 1) % 3]);
            let around = &round[starts[from as usize] as usize..starts[from as usize + 1] as usize];
            for &other in around {
                if other as usize <= face {
                    continue;
                }
                let o = mesh.triangles[other as usize];
                let Some(at) = o.iter().position(|&v| v == to) else {
                    continue;
                };
                // The other runs the edge from `to` to `from`.
                if o[(at + 1) % 3] != from || out[other as usize][at].is_some() {
                    continue;
                }
                out[face][k] = Some(other);
                out[other as usize][at] = Some(face as u32);
                break;
            }
        }
    }
    out
}

/// The triangles of each piece, by index, in the reference's order.
fn patches(mesh: &Mesh, stop_after: usize) -> Vec<Vec<u32>> {
    let neighbours = neighbours(mesh);
    let mut seen = vec![false; mesh.triangles.len()];
    let mut out = Vec::new();
    let mut seed = 0;
    let mut stack: Vec<u32> = Vec::new();
    while out.len() < stop_after {
        while seed < seen.len() && seen[seed] {
            seed += 1;
        }
        if seed == seen.len() {
            break;
        }
        let mut faces = vec![seed as u32];
        seen[seed] = true;
        stack.push(seed as u32);
        while let Some(face) = stack.pop() {
            for next in neighbours[face as usize].into_iter().flatten() {
                if !seen[next as usize] {
                    seen[next as usize] = true;
                    faces.push(next);
                    stack.push(next);
                }
            }
        }
        out.push(faces);
    }
    out
}

/// Whether the mesh is more than one piece.
pub fn is_splittable(mesh: &Mesh) -> bool {
    patches(mesh, 2).len() > 1
}

/// The pieces, each a mesh of its own with only the corners it uses. A
/// piece that encloses a negative volume is turned the right way out, as
/// the reference turns one ("some source mesh parts may be incorrectly
/// oriented").
pub fn split(mesh: &Mesh) -> Vec<Mesh> {
    // For each corner, the piece that last took it and where it is there.
    let mut image: Vec<(usize, u32)> = vec![(usize::MAX, 0); mesh.vertices.len()];
    patches(mesh, usize::MAX)
        .into_iter()
        .enumerate()
        .map(|(part, faces)| {
            let mut piece = Mesh::default();
            for face in faces {
                let t = mesh.triangles[face as usize].map(|v| {
                    let v = v as usize;
                    if image[v].0 != part {
                        image[v] = (part, piece.vertices.len() as u32);
                        piece.vertices.push(mesh.vertices[v]);
                    }
                    image[v].1
                });
                piece.triangles.push(t);
            }
            if piece.signed_volume() < 0.0 {
                for t in &mut piece.triangles {
                    t.swap(1, 2);
                }
            }
            piece
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::{cuboid, inverted};

    fn mesh(parts: &[Vec<crate::Triangle>]) -> Mesh {
        Mesh::from_triangle_soup(parts.iter().flatten().copied())
    }

    #[test]
    fn one_cube_is_one_piece() {
        let cube = mesh(&[cuboid([0.0; 3], [1.0; 3])]);
        assert!(!is_splittable(&cube));
        let pieces = split(&cube);
        assert_eq!(pieces.len(), 1);
        // The same solid; its triangles in the order the walk takes them,
        // the first first.
        assert_eq!(pieces[0].triangles.len(), 12);
        assert_eq!(pieces[0].vertices.len(), 8);
        assert_eq!(pieces[0].triangle(0), cube.triangle(0));
        assert_eq!(pieces[0].signed_volume(), cube.signed_volume());
        assert!(neighbours(&cube).iter().flatten().all(Option::is_some));
    }

    #[test]
    fn apart_and_touching_at_a_corner_are_two_pieces() {
        // Two cubes that touch at one corner share a vertex, not an edge.
        let both = mesh(&[cuboid([0.0; 3], [1.0; 3]), cuboid([1.0; 3], [3.0; 3])]);
        assert!(is_splittable(&both));
        let pieces = split(&both);
        assert_eq!(pieces.len(), 2);
        assert_eq!(
            pieces.iter().map(|p| p.triangles.len()).collect::<Vec<_>>(),
            [12, 12]
        );
        assert_eq!(
            pieces.iter().map(|p| p.vertices.len()).collect::<Vec<_>>(),
            [8, 8]
        );
        // In the order of their first triangle.
        assert_eq!(pieces[0].bounds(), Some(([0.0; 3], [1.0; 3])));
        assert_eq!(pieces[1].bounds(), Some(([1.0; 3], [3.0; 3])));
        assert!((pieces[1].signed_volume() - 8.0).abs() < 1e-9);
    }

    #[test]
    fn a_piece_inside_out_is_turned_the_right_way() {
        let both = mesh(&[
            cuboid([0.0; 3], [1.0; 3]),
            inverted(cuboid([5.0; 3], [6.0; 3])),
        ]);
        let pieces = split(&both);
        assert_eq!(pieces.len(), 2);
        assert!(pieces.iter().all(|p| p.signed_volume() > 0.0));
    }

    #[test]
    fn edges_running_the_same_way_do_not_join() {
        // Two triangles over one edge, both wound alike: not a closed
        // surface across it.
        let soup = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, -1.0, 0.0]],
        ];
        let m = Mesh::from_triangle_soup(soup);
        assert!(is_splittable(&m));
        let soup = [
            [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            [[1.0, 0.0, 0.0], [0.0, 0.0, 0.0], [0.0, -1.0, 0.0]],
        ];
        assert!(!is_splittable(&Mesh::from_triangle_soup(soup)));
    }
}
