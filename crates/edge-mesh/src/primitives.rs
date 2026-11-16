//! Procedural solids for tests and synthetic benchmarks.

use crate::{Mesh, Triangle};

/// An axis-aligned box as twelve outward-facing triangles.
pub fn cuboid(min: [f32; 3], max: [f32; 3]) -> Vec<Triangle> {
    // Corner `i` takes max on axis `a` when bit `a` of `i` is set.
    let corner = |i: usize| -> [f32; 3] {
        std::array::from_fn(|axis| {
            if i >> axis & 1 == 1 {
                max[axis]
            } else {
                min[axis]
            }
        })
    };
    const FACES: [[usize; 3]; 12] = [
        [0, 2, 3],
        [0, 3, 1], // -z
        [4, 5, 7],
        [4, 7, 6], // +z
        [0, 1, 5],
        [0, 5, 4], // -y
        [2, 6, 7],
        [2, 7, 3], // +y
        [0, 4, 6],
        [0, 6, 2], // -x
        [1, 3, 7],
        [1, 7, 5], // +x
    ];
    FACES.iter().map(|face| face.map(corner)).collect()
}

/// A flat region: its outline (anticlockwise) and holes (clockwise), in mm.
pub type FlatRegion = (Vec<[f32; 2]>, Vec<Vec<[f32; 2]>>);

/// A flat region raised into a solid from `z0` to `z1` (Emboss.cpp
/// `polygons2model` with a `ProjectZ`): each region an outline
/// (anticlockwise) and its holes (clockwise), corners in mm. The caps are
/// cut into triangles by ear cutting, every corner kept, and each edge of
/// a ring is a side of two triangles; the corners are shared, so the solid
/// is closed. The reference cuts its caps with a constrained Delaunay
/// triangulation instead: the same solid, other triangles. `None` when a
/// cap cannot be cut whole.
pub fn prism(regions: &[FlatRegion], z0: f32, z1: f32) -> Option<Mesh> {
    let mut corners: Vec<[f32; 2]> = Vec::new();
    let mut rings: Vec<(Vec<u32>, Vec<Vec<u32>>)> = Vec::new();
    let mut add = |ring: &[[f32; 2]]| -> Vec<u32> {
        ring.iter()
            .map(|p| {
                corners.push(*p);
                u32::try_from(corners.len() - 1).expect("prism exceeds u32 vertex indices")
            })
            .collect()
    };
    for (outline, holes) in regions {
        let outline = add(outline);
        let holes = holes.iter().map(|hole| add(hole)).collect();
        rings.push((outline, holes));
    }
    let xy = |v: u32| corners[v as usize].map(f64::from);
    let mut caps = Vec::new();
    for (outline, holes) in &rings {
        if !crate::earcut::triangulate(outline, holes, &xy, &mut caps) {
            return None;
        }
    }
    let count = u32::try_from(corners.len()).expect("prism exceeds u32 vertex indices");
    let mut mesh = Mesh {
        vertices: corners.iter().map(|[x, y]| [*x, *y, z0]).collect(),
        triangles: Vec::with_capacity(2 * caps.len() + 2 * corners.len()),
    };
    mesh.vertices
        .extend(corners.iter().map(|[x, y]| [*x, *y, z1]));
    // The top faces up as cut, the bottom down.
    for &[a, b, c] in &caps {
        mesh.triangles.push([a + count, b + count, c + count]);
        mesh.triangles.push([a, c, b]);
    }
    // The sides, the region always to the left of each edge.
    for (outline, holes) in &rings {
        for ring in std::iter::once(outline).chain(holes) {
            for (k, &cur) in ring.iter().enumerate() {
                let prev = ring[(k + ring.len() - 1) % ring.len()];
                mesh.triangles.push([prev, cur, cur + count]);
                mesh.triangles.push([prev, cur + count, prev + count]);
            }
        }
    }
    Some(mesh)
}

/// Reverses the winding so a solid reads as a cavity inside another solid.
pub fn inverted(mut triangles: Vec<Triangle>) -> Vec<Triangle> {
    for triangle in &mut triangles {
        triangle.swap(1, 2);
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuboid_faces_point_outward() {
        let centre = [0.5f32; 3];
        for [a, b, c] in cuboid([0.0; 3], [1.0; 3]) {
            let (u, v) = (sub(b, a), sub(c, a));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let outward = sub(a, centre);
            let dot: f32 = (0..3).map(|i| normal[i] * outward[i]).sum();
            assert!(dot > 0.0, "triangle {a:?} {b:?} {c:?} faces inward");
        }
    }

    #[test]
    fn a_prism_with_a_hole_is_closed_and_faces_out() {
        let outline = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
        let mesh = prism(&[(outline, vec![hole])], 1.0, 3.0).unwrap();
        assert!((mesh.signed_volume() - 96.0 * 2.0).abs() < 1e-4);
        // Every edge is met by exactly two triangles, once each way.
        let mut edges = std::collections::BTreeMap::new();
        for t in &mesh.triangles {
            for k in 0..3 {
                *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        for (&(a, b), &n) in &edges {
            assert_eq!(n, 1);
            assert_eq!(edges.get(&(b, a)), Some(&1));
        }
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| a[i] - b[i])
    }
}
