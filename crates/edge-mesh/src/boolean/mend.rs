//! What keeps a mesh that is closed in space from being closed in its
//! edges, put right without moving its surface: the two faults the
//! library refuses an operand for that the reference gets past with its
//! fallback (MeshBoolean.cpp: `mfd::make_boolean` returns false on a
//! status other than `NoError`, and the caller goes on to mcut, which cuts
//! surfaces and does not ask for closed ones).
//!
//! - A triangle written twice, the same corners the same way round (a
//!   file that repeats a facet): one is kept.
//! - A T-junction: a corner of one triangle lying on an edge of its
//!   neighbour, where a modeller split one side of a seam and not the
//!   other. The edge is split at the corner, so the two sides meet edge to
//!   edge.
//!
//! Anything else that leaves an edge open (a hole, a missing face) is
//! left as it is, and the mesh stays refused: a hole is not closed by
//! guessing what was there.

use rustc_hash::{FxHashMap, FxHashSet};

use super::check;
use crate::Mesh;

/// How far from an edge, as a share of its length, a corner may be and
/// still be taken as lying on it: a T-junction written in single
/// precision is rarely on the line exactly.
const ON_EDGE: f64 = 1e-5;

/// The mesh with its repeated triangles and T-junctions put right, when
/// that closes it; `None` when it is still open.
pub(crate) fn mend(mesh: &Mesh) -> Option<Mesh> {
    // Corners at one place made one, as the import's weld does.
    let mut mesh = Mesh::from_triangle_soup((0..mesh.triangles.len()).map(|t| mesh.triangle(t)));
    drop_repeats(&mut mesh);
    if !check::is_closed(&mesh) {
        split_t_junctions(&mut mesh);
    }
    check::is_closed(&mesh).then_some(mesh)
}

/// One of each triangle written more than once the same way round.
fn drop_repeats(mesh: &mut Mesh) {
    let mut seen: FxHashSet<[u32; 3]> = FxHashSet::default();
    mesh.triangles.retain(|t| {
        // The same triangle whichever corner it starts from.
        let k = (0..3).min_by_key(|&k| t[k]).unwrap_or(0);
        seen.insert([t[k], t[(k + 1) % 3], t[(k + 2) % 3]])
    });
}

/// An edge of a triangle (its place, 0 to 2) and the corners to put on
/// it, each with where it is along the edge.
type EdgeSplit = (usize, Vec<(f64, u32)>);

fn key(from: u32, to: u32) -> u64 {
    (u64::from(from) << 32) | u64::from(to)
}

/// Where `p` is along the edge from `a` to `b` (0 at `a`, 1 at `b`), when
/// it lies on the edge strictly between its ends.
fn along(a: [f32; 3], b: [f32; 3], p: [f32; 3]) -> Option<f64> {
    let d: [f64; 3] = std::array::from_fn(|i| f64::from(b[i]) - f64::from(a[i]));
    let q: [f64; 3] = std::array::from_fn(|i| f64::from(p[i]) - f64::from(a[i]));
    let length2 = d.iter().map(|c| c * c).sum::<f64>();
    if length2 == 0.0 {
        return None;
    }
    let t = q.iter().zip(&d).map(|(q, d)| q * d).sum::<f64>() / length2;
    if t <= ON_EDGE || t >= 1.0 - ON_EDGE {
        return None;
    }
    let off2: f64 = (0..3).map(|i| (q[i] - t * d[i]).powi(2)).sum();
    (off2 <= ON_EDGE * ON_EDGE * length2).then_some(t)
}

/// Splits each open edge at the corners of other open edges that lie on
/// it, the triangle along it made a fan from its third corner. A few
/// rounds, as a split can uncover the next.
fn split_t_junctions(mesh: &mut Mesh) {
    for _ in 0..4 {
        let mut edges: FxHashMap<u64, usize> = FxHashMap::default();
        for (at, t) in mesh.triangles.iter().enumerate() {
            for k in 0..3 {
                edges.insert(key(t[k], t[(k + 1) % 3]), at);
            }
        }
        let open: Vec<(u32, u32, usize)> = (edges.iter())
            .filter(|(edge, _)| !edges.contains_key(&edge.rotate_right(32)))
            .map(|(edge, at)| ((edge >> 32) as u32, *edge as u32, *at))
            .collect();
        if open.is_empty() {
            return;
        }
        // The corners on open edges, the only ones a T-junction can be at.
        let mut corners: Vec<u32> = open.iter().flat_map(|&(a, b, _)| [a, b]).collect();
        corners.sort_unstable();
        corners.dedup();
        // Every open edge against every such corner: a mesh with many is
        // not one a T-junction or two left open, and is left alone.
        if open.len().saturating_mul(corners.len()) > 50_000_000 {
            return;
        }
        // For each triangle, the corners to put on each of its edges.
        let mut splits: FxHashMap<usize, Vec<EdgeSplit>> = FxHashMap::default();
        let mut sorted = open.clone();
        sorted.sort_unstable();
        for (a, b, at) in sorted {
            let (pa, pb) = (mesh.vertices[a as usize], mesh.vertices[b as usize]);
            let mut on: Vec<(f64, u32)> = (corners.iter())
                .filter(|&&v| v != a && v != b)
                .filter_map(|&v| along(pa, pb, mesh.vertices[v as usize]).map(|t| (t, v)))
                .collect();
            if on.is_empty() {
                continue;
            }
            on.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
            let t = mesh.triangles[at];
            let side = (0..3)
                .find(|&k| t[k] == a && t[(k + 1) % 3] == b)
                .unwrap_or(0);
            splits.entry(at).or_default().push((side, on));
        }
        if splits.is_empty() {
            return;
        }
        let mut order: Vec<usize> = splits.keys().copied().collect();
        order.sort_unstable();
        for at in order {
            let t = mesh.triangles[at];
            let mut on: [Vec<u32>; 3] = Default::default();
            for (side, corners) in &splits[&at] {
                on[*side] = corners.iter().map(|(_, v)| *v).collect();
            }
            let mut cover = Vec::new();
            split_triangle(t, on, &mut cover);
            mesh.triangles[at] = cover[0];
            mesh.triangles.extend_from_slice(&cover[1..]);
        }
    }
}

/// Triangles covering triangle `t` with the corners `on` each of its
/// edges (edge `k` from corner `k` to the next, its corners in order along
/// it) made corners of theirs: `t` cut in two from a corner on an edge to
/// the opposite corner, and each half the same, until no edge has one.
fn split_triangle(t: [u32; 3], on: [Vec<u32>; 3], out: &mut Vec<[u32; 3]>) {
    let Some(k) = (0..3).find(|&k| !on[k].is_empty()) else {
        out.push(t);
        return;
    };
    let [ab, bc, ca] = [k, (k + 1) % 3, (k + 2) % 3].map(|side| on[side].clone());
    let (a, b, c) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
    let (v, before, after) = (ab[0], Vec::new(), ab[1..].to_vec());
    split_triangle([a, v, c], [before, Vec::new(), ca], out);
    split_triangle([v, b, c], [after, bc, Vec::new()], out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::cuboid;

    #[test]
    fn a_repeated_triangle_and_a_t_junction_are_put_right() {
        let cube = Mesh::from_triangle_soup(cuboid([0.0; 3], [10.0; 3]));
        let mut repeated = cube.clone();
        repeated.triangles.push(repeated.triangles[3]);
        assert!(!check::is_closed(&repeated));
        let mended = mend(&repeated).unwrap();
        assert_eq!(mended.triangles.len(), 12);

        // The edge from (0, 0, 10) to (10, 0, 10) split on the front's side
        // only.
        let mut t = cube.clone();
        let find = |p: [f32; 3]| t.vertices.iter().position(|v| *v == p).unwrap() as u32;
        let (a, b) = (find([0.0, 0.0, 10.0]), find([10.0, 0.0, 10.0]));
        let at = (t.triangles.iter())
            .position(|tr| {
                (0..3).any(|k| tr[k] == a && tr[(k + 1) % 3] == b)
                    || (0..3).any(|k| tr[k] == b && tr[(k + 1) % 3] == a)
            })
            .unwrap();
        let tr = t.triangles[at];
        let k = (0..3)
            .find(|&k| (tr[k] == a && tr[(k + 1) % 3] == b) || (tr[k] == b && tr[(k + 1) % 3] == a))
            .unwrap();
        let mid = t.vertices.len() as u32;
        t.vertices.push([5.0, 0.0, 10.0]);
        t.triangles[at] = [tr[k], mid, tr[(k + 2) % 3]];
        t.triangles.push([mid, tr[(k + 1) % 3], tr[(k + 2) % 3]]);
        assert_eq!(check::defects(&t).open_edges, 3);
        let mended = mend(&t).unwrap();
        assert!(check::defects(&mended).is_clean());
        assert!((mended.signed_volume() - 1000.0).abs() < 1e-9);

        // A hole stays a hole.
        let mut open = cube;
        open.triangles.truncate(10);
        assert!(mend(&open).is_none());
    }
}
