//! Corners closer than a tolerance made one: what a boolean can leave a
//! hair apart (a fraction of a micron) and single precision then folds
//! into a triangle of no area once the mesh is moved far from its origin
//! (a part placed on the bed).

use std::collections::HashMap;

use crate::Mesh;

/// The mesh with corners nearer than `tolerance` made one, the triangles
/// that then lose a corner taken out, and two that then lie on one another
/// back to back taken out too. None when that would leave the mesh open,
/// the mesh then to be kept as it was. Deterministic: corners are joined to
/// the first of them.
pub fn weld_close(mesh: &Mesh, tolerance: f32) -> Option<Mesh> {
    let n = mesh.vertices.len();
    let cell = |v: [f32; 3]| v.map(|c| (c / tolerance).floor() as i64);
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for (at, v) in mesh.vertices.iter().enumerate() {
        grid.entry(cell(*v)).or_default().push(at as u32);
    }
    // Each corner to the first one within the tolerance (union-find).
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn root(parent: &mut [u32], mut at: u32) -> u32 {
        while parent[at as usize] != at {
            parent[at as usize] = parent[parent[at as usize] as usize];
            at = parent[at as usize];
        }
        at
    }
    let close = |a: [f32; 3], b: [f32; 3]| {
        (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>() <= tolerance * tolerance
    };
    for (at, v) in mesh.vertices.iter().enumerate() {
        let [x, y, z] = cell(*v);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let Some(others) = grid.get(&[x + dx, y + dy, z + dz]) else {
                        continue;
                    };
                    for &other in others {
                        if other as usize >= at || !close(*v, mesh.vertices[other as usize]) {
                            continue;
                        }
                        let (a, b) = (root(&mut parent, at as u32), root(&mut parent, other));
                        let (low, high) = (a.min(b), a.max(b));
                        parent[high as usize] = low;
                    }
                }
            }
        }
    }
    let mut triangles: Vec<[u32; 3]> = Vec::with_capacity(mesh.triangles.len());
    for t in &mesh.triangles {
        let t = t.map(|v| root(&mut parent, v));
        if t[0] != t[1] && t[1] != t[2] && t[0] != t[2] {
            triangles.push(t);
        }
    }
    // Back to back pairs: a fold the collapse made, no surface at all.
    let key = |t: &[u32; 3]| {
        let mut sorted = *t;
        sorted.sort_unstable();
        sorted
    };
    let mut count: HashMap<[u32; 3], usize> = HashMap::new();
    for t in &triangles {
        *count.entry(key(t)).or_default() += 1;
    }
    triangles.retain(|t| count[&key(t)] == 1);
    // The corners still used, in their order.
    let mut remap = vec![u32::MAX; n];
    let mut out = Mesh::default();
    for t in &triangles {
        for &v in t {
            if remap[v as usize] == u32::MAX {
                remap[v as usize] = out.vertices.len() as u32;
                out.vertices.push(mesh.vertices[v as usize]);
            }
        }
    }
    out.triangles = triangles
        .iter()
        .map(|t| t.map(|v| remap[v as usize]))
        .collect();
    crate::cut::is_closed(&out).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::cuboid;

    #[test]
    fn a_corner_a_hair_from_another_is_made_one_and_the_mesh_stays_closed() {
        // A box whose one top corner is split in two a hair apart, with a
        // sliver between them: what a boolean can leave.
        let mut mesh = Mesh::from_triangle_soup(cuboid([0.0; 3], [10.0; 3]));
        let top = mesh
            .vertices
            .iter()
            .position(|v| *v == [10.0, 10.0, 10.0])
            .unwrap() as u32;
        let twin = mesh.vertices.len() as u32;
        mesh.vertices.push([10.0, 10.0 - 4e-7, 10.0]);
        // Hand one triangle at the corner the twin, and close the gap with
        // a sliver.
        let (owner, slot) = mesh
            .triangles
            .iter()
            .enumerate()
            .find_map(|(at, t)| t.iter().position(|v| *v == top).map(|slot| (at, slot)))
            .unwrap();
        let old = mesh.triangles[owner];
        mesh.triangles[owner][slot] = twin;
        let next = old[(slot + 1) % 3];
        let previous = old[(slot + 2) % 3];
        // The edges the twin now has unmatched are closed by two slivers.
        mesh.triangles.push([top, twin, previous]);
        mesh.triangles.push([twin, top, next]);
        assert!(crate::cut::is_closed(&mesh));
        // Moved far out, single precision folds the slivers flat.
        let mut moved = Mesh::default();
        moved.append(&mesh, &crate::Transform::translation([200.0, 200.0, 0.0]));
        assert!(crate::cut::flat_triangles(&moved) > 0);
        let welded = weld_close(&mesh, 1e-4).unwrap();
        let mut moved = Mesh::default();
        moved.append(&welded, &crate::Transform::translation([200.0, 200.0, 0.0]));
        assert_eq!(crate::cut::flat_triangles(&moved), 0);
        assert!(crate::cut::is_closed(&welded));
        assert!((welded.signed_volume() - 1000.0).abs() < 1e-3);
        assert_eq!(welded.triangles.len(), 12);
    }
}
