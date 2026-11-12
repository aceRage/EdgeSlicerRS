//! What a solid's mesh is checked for: every edge run once each way by
//! exactly two triangles (closed and consistently wound), and no triangle
//! of zero area. Used on every boolean's result before it is returned,
//! and by the tests.

use rustc_hash::FxHashMap;

use crate::Mesh;

/// What is wrong with a mesh as the surface of a solid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Defects {
    /// Directed edges with no triangle running them the other way.
    pub open_edges: usize,
    /// Directed edges run by more than one triangle the same way: an edge
    /// of more than two triangles, or two neighbours wound against each
    /// other.
    pub doubled_edges: usize,
    /// Triangles whose corners are on one line (or at one place).
    pub zero_area: usize,
    /// Triangles that use a corner twice, or one that is not there.
    pub malformed: usize,
}

impl Defects {
    pub fn is_clean(&self) -> bool {
        *self == Defects::default()
    }
}

/// The mesh's defects. An empty mesh has none.
pub fn defects(mesh: &Mesh) -> Defects {
    let mut out = Defects::default();
    let count = mesh.vertices.len() as u32;
    let mut edges: Vec<u64> = Vec::with_capacity(mesh.triangles.len() * 3);
    for (at, t) in mesh.triangles.iter().enumerate() {
        if t.iter().any(|&v| v >= count) || t[0] == t[1] || t[1] == t[2] || t[2] == t[0] {
            out.malformed += 1;
            continue;
        }
        if is_zero_area(mesh, at) {
            out.zero_area += 1;
        }
        for k in 0..3 {
            edges.push(key(t[k], t[(k + 1) % 3]));
        }
    }
    edges.sort_unstable();
    for (at, &edge) in edges.iter().enumerate() {
        if at > 0 && edges[at - 1] == edge {
            out.doubled_edges += 1;
        }
        let reverse = edge.rotate_right(32);
        if edges.binary_search(&reverse).is_err() {
            out.open_edges += 1;
        }
    }
    out
}

/// Whether every edge is run once each way by exactly two triangles.
pub fn is_closed(mesh: &Mesh) -> bool {
    let d = defects(mesh);
    d.open_edges == 0 && d.doubled_edges == 0 && d.malformed == 0
}

fn key(from: u32, to: u32) -> u64 {
    (u64::from(from) << 32) | u64::from(to)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f64::from(a[i]) - f64::from(b[i]))
}

fn cross(u: [f64; 3], v: [f64; 3]) -> [f64; 3] {
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

/// Whether a triangle has no area, as the slicer will compute it.
pub fn is_zero_area(mesh: &Mesh, triangle: usize) -> bool {
    let [a, b, c] = mesh.triangle(triangle);
    cross(sub(b, a), sub(c, a)) == [0.0; 3]
}

/// Turns needles away: a triangle of zero area whose three corners are
/// apart, one of them on the edge between the other two, is flipped with
/// its neighbour across that edge, which leaves two triangles with area
/// covering the neighbour (Manifold's `SwapDegenerates` does the same in
/// double precision; rounding to single can make new ones). Returns how
/// many were flipped.
pub(crate) fn flip_needles(mesh: &mut Mesh) -> usize {
    let needles: Vec<usize> = (0..mesh.triangles.len())
        .filter(|&t| is_zero_area(mesh, t))
        .collect();
    if needles.is_empty() {
        return 0;
    }
    let mut owner: FxHashMap<u64, usize> = FxHashMap::default();
    for (at, t) in mesh.triangles.iter().enumerate() {
        for k in 0..3 {
            owner.insert(key(t[k], t[(k + 1) % 3]), at);
        }
    }
    let mut flipped = 0;
    for needle in needles {
        if !is_zero_area(mesh, needle) {
            continue;
        }
        let t = mesh.triangles[needle];
        let p = t.map(|v| mesh.vertices[v as usize]);
        if p[0] == p[1] || p[1] == p[2] || p[2] == p[0] {
            continue;
        }
        // The longest edge is the one the third corner lies on.
        let length = |k: usize| {
            let d = sub(p[(k + 1) % 3], p[k]);
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
        };
        let k = (0..3)
            .max_by(|&x, &y| length(x).total_cmp(&length(y)))
            .unwrap_or(0);
        let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
        let Some(&other) = owner.get(&key(b, a)) else {
            continue;
        };
        let o = mesh.triangles[other];
        let Some(d) = o.iter().copied().find(|&v| v != a && v != b) else {
            continue;
        };
        if owner.contains_key(&key(c, d)) || owner.contains_key(&key(d, c)) {
            continue;
        }
        let first = [a, d, c];
        let second = [d, b, c];
        let before = (mesh.triangles[needle], mesh.triangles[other]);
        mesh.triangles[needle] = first;
        mesh.triangles[other] = second;
        if is_zero_area(mesh, needle) || is_zero_area(mesh, other) {
            (mesh.triangles[needle], mesh.triangles[other]) = before;
            continue;
        }
        for (from, to) in [(a, b), (b, c), (c, a), (b, a), (a, d), (d, b)] {
            owner.remove(&key(from, to));
        }
        for (at, t) in [(needle, first), (other, second)] {
            for k in 0..3 {
                owner.insert(key(t[k], t[(k + 1) % 3]), at);
            }
        }
        flipped += 1;
    }
    flipped
}

/// Whether two triangles of the mesh meet anywhere but along the edges
/// and at the corners they share: an exact test (the library's). For
/// tests and for callers that can afford it on a large mesh; a closed
/// mesh is asked for.
pub fn self_intersects(mesh: &Mesh) -> bool {
    let gl = super::to_library(mesh);
    let manifold = manifold_rust::manifold::Manifold::from_mesh_gl64(&gl);
    manifold.has_self_intersections()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::cuboid;

    #[test]
    fn a_cube_is_clean_and_an_open_box_is_not() {
        let mut cube = Mesh::from_triangle_soup(cuboid([0.0; 3], [1.0; 3]));
        assert!(defects(&cube).is_clean());
        cube.triangles.pop();
        let d = defects(&cube);
        assert_eq!(d.open_edges, 3);
        assert!(!is_closed(&cube));
    }

    #[test]
    fn a_needle_is_flipped_away() {
        // A square split into a needle on its bottom edge and the rest.
        let mut mesh = Mesh {
            vertices: vec![
                [0.0, 0.0, 0.0],
                [2.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
            // The needle 0-1-2 and its neighbour across 0-1.
            triangles: vec![[0, 1, 2], [1, 0, 3]],
        };
        assert_eq!(defects(&mesh).zero_area, 1);
        assert_eq!(flip_needles(&mut mesh), 1);
        assert_eq!(defects(&mesh).zero_area, 0);
        assert_eq!(mesh.triangles, vec![[0, 3, 2], [3, 1, 2]]);
    }
}
