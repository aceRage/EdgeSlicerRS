//! Cutting a mesh in two by a plane, each half closed with a cap over the
//! cut: the reference's `cut_mesh` and `triangulate_slice`
//! (libslic3r/TriangleMeshSlicer.cpp), which `Cut::perform_with_plane`
//! (CutUtils.cpp) runs on each part of an object.
//!
//! The reference intersects each triangle on its own, in scaled integer
//! coordinates, triangulates the cap from the lines it found
//! (`make_expolygons_simple`, then `triangulate_expolygons_3d`) and then
//! looks for each corner of each cap triangle among the cut's vertices by
//! nearness, adding a vertex where it finds none. A cap corner it does not
//! find, or finds twice, leaves the half with open edges, which is why the
//! reference asks on Windows whether to repair what it cut ("non-manifold
//! edges be caused by cut tool, do you want to fix it now?"). This cut is
//! built so that cannot happen:
//!
//! - a corner within [`ON_PLANE`] of the plane is taken as on it, so the
//!   plane through a corner, an edge or a face is one case, not a near miss;
//! - a crossed edge is cut once, and both triangles on it take that one new
//!   vertex, so the two sides of every cut edge meet;
//! - the cap is laid over the edges the cut opened (an edge one half has and
//!   the other has the other way round), followed round as loops, and
//!   triangulated with no corner but those loops' own, every one of them
//!   used, so each cap edge is a cut edge run the other way;
//! - no triangle of no area is made: a split never puts three corners on a
//!   line, and the cap's ears are taken only where they turn.
//!
//! What it does not close: where the cut opens a chain of edges that does
//! not come round (a mesh that was open where the plane crosses it), no cap
//! is laid there, and [`Cut::open`] counts it. The reference's cut of such
//! a mesh caps what its polygon making makes of the lines; here an open
//! mesh stays open where it was open.

use rustc_hash::FxHashMap;

use crate::{Mesh, Transform};

/// How near the plane a corner is taken as on it, in mm: the reference's
/// `EPSILON` (libslic3r.h), with which `cut_mesh` compares heights.
pub const ON_PLANE: f64 = 1e-4;

/// Where a triangle of a half came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The triangle of the mesh that was cut, by its place.
    pub triangle: u32,
    /// Whether it is that triangle whole, its corners in the same order
    /// (what was painted on it then still fits it), or a piece of it.
    pub whole: bool,
}

/// One side of a cut: its mesh, and for each of its triangles where it
/// came from; none for a triangle of the cap.
#[derive(Clone, Debug, Default)]
pub struct Half {
    pub mesh: Mesh,
    pub from: Vec<Option<Origin>>,
}

impl Half {
    pub fn is_empty(&self) -> bool {
        self.mesh.triangles.is_empty()
    }
}

/// What a cut made: the part above the plane and the part under it.
#[derive(Clone, Debug, Default)]
pub struct Cut {
    pub upper: Half,
    pub lower: Half,
    /// How many chains of cut edges were left without a cap: the mesh was
    /// open where the plane crosses it, or a loop could not be laid flat.
    pub open: usize,
}

/// Cuts `mesh` in two. `to_plane` takes the mesh's coordinates to those of
/// the cut, in which the plane is `z = 0` and the upper half is where `z`
/// is greater. `faces` are the heights there of the lower half's cut face
/// and the upper half's: `[0, 0]` for a plain cut; apart, what lies between
/// them is cut away (the reference's cut thickness, `curved_cut_thickness_faces`,
/// which cuts at each face in turn as this does). `back` takes the cut's
/// coordinates to those the halves are given in.
pub fn cut(mesh: &Mesh, to_plane: &Transform, faces: [f64; 2], back: &Transform) -> Cut {
    let points: Vec<[f64; 3]> = mesh
        .vertices
        .iter()
        .map(|v| to_plane.apply(v.map(f64::from)))
        .collect();
    // Worked out with the triangles turned the right way out in the cut's
    // coordinates; turned again for `back`.
    let inward = to_plane.is_mirrored();
    let outward = back.is_mirrored();
    if faces[0] == faces[1] {
        let (upper, lower, open) = halves(&points, &mesh.triangles, faces[0], inward);
        Cut {
            upper: finish(upper, back, outward),
            lower: finish(lower, back, outward),
            open,
        }
    } else {
        let (low, high) = (faces[0].min(faces[1]), faces[0].max(faces[1]));
        let (_, lower, open_low) = halves(&points, &mesh.triangles, low, inward);
        let (upper, _, open_high) = halves(&points, &mesh.triangles, high, inward);
        Cut {
            upper: finish(upper, back, outward),
            lower: finish(lower, back, outward),
            open: open_low + open_high,
        }
    }
}

/// The lines the plane cuts the mesh along, as pairs of points in the
/// cut's coordinates (`z = 0`): what the cut's face will be. Empty when the
/// plane misses the mesh. Quick enough to be asked for while the plane is
/// dragged.
pub fn section(mesh: &Mesh, to_plane: &Transform) -> Vec<[[f64; 3]; 2]> {
    let points: Vec<[f64; 3]> = mesh
        .vertices
        .iter()
        .map(|v| to_plane.apply(v.map(f64::from)))
        .collect();
    let side = |p: &[f64; 3]| side_of(p[2] - 0.0);
    let on = |p: [f64; 3]| [p[0], p[1], 0.0];
    let mut out = Vec::new();
    for t in &mesh.triangles {
        let c = t.map(|v| points[v as usize]);
        let s = c.each_ref().map(side);
        if s.iter().all(|s| *s == 0) {
            continue;
        }
        let mut found: Vec<[f64; 3]> = Vec::with_capacity(2);
        for k in 0..3 {
            let (a, b) = (k, (k + 1) % 3);
            if s[a] == 0 {
                found.push(on(c[a]));
            } else if s[a] * s[b] < 0 {
                found.push(crossing(c[a], c[b], 0.0));
            }
        }
        if found.len() == 2 && found[0] != found[1] {
            out.push([found[0], found[1]]);
        }
    }
    out
}

/// Whether every edge of a mesh is run once each way: a closed surface,
/// its triangles all turned alike.
pub fn is_closed(mesh: &Mesh) -> bool {
    let mut count: FxHashMap<(u32, u32), i64> = FxHashMap::default();
    for t in &mesh.triangles {
        for k in 0..3 {
            let (a, b) = (t[k], t[(k + 1) % 3]);
            if a == b {
                return false;
            }
            let (key, step) = if a < b { ((a, b), 1) } else { ((b, a), -1) };
            *count.entry(key).or_default() += step;
        }
    }
    count.values().all(|n| *n == 0)
}

/// How many triangles of a mesh have no area (corners on a line, or two
/// the same).
pub fn flat_triangles(mesh: &Mesh) -> usize {
    (0..mesh.triangles.len())
        .filter(|at| {
            let [a, b, c] = mesh.triangle(*at).map(|p| p.map(f64::from));
            let u = sub(b, a);
            let v = sub(c, a);
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            n == [0.0; 3]
        })
        .count()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn side_of(height: f64) -> i8 {
    if height.abs() <= ON_PLANE {
        0
    } else if height > 0.0 {
        1
    } else {
        -1
    }
}

/// Where the edge from `a` to `b` meets the plane at height `z`, the same
/// point whichever way round the edge is given.
fn crossing(a: [f64; 3], b: [f64; 3], z: f64) -> [f64; 3] {
    let (a, b) = if (a[0], a[1], a[2]) <= (b[0], b[1], b[2]) {
        (a, b)
    } else {
        (b, a)
    };
    let t = (z - a[2]) / (b[2] - a[2]);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, z]
}

/// A half as worked out: triangles by index into the shared points.
struct Raw {
    triangles: Vec<[u32; 3]>,
    from: Vec<Option<Origin>>,
}

/// Cuts at height `z`: the triangles above, the triangles under, each
/// with its cap, over one list of points (the mesh's, each within
/// [`ON_PLANE`] of the plane put on it, then those the cut made). Answers
/// the points with the halves, and how many chains were left open.
fn halves(
    points: &[[f64; 3]],
    triangles: &[[u32; 3]],
    z: f64,
    inward: bool,
) -> (Sided, Sided, usize) {
    let mut points: Vec<[f64; 3]> = points.to_vec();
    let mut sides: Vec<i8> = Vec::with_capacity(points.len());
    for p in &mut points {
        let s = side_of(p[2] - z);
        if s == 0 {
            p[2] = z;
        }
        sides.push(s);
    }
    let mut upper = Raw {
        triangles: Vec::new(),
        from: Vec::new(),
    };
    let mut lower = Raw {
        triangles: Vec::new(),
        from: Vec::new(),
    };
    // The vertex made on each crossed edge, by the edge's corners in order.
    let mut made: FxHashMap<(u32, u32), u32> = FxHashMap::default();
    for (index, t) in triangles.iter().enumerate() {
        let index = index as u32;
        if t[0] == t[1] || t[1] == t[2] || t[2] == t[0] {
            continue;
        }
        let t = if inward { [t[0], t[2], t[1]] } else { *t };
        let s = t.map(|v| sides[v as usize]);
        let whole = Some(Origin {
            triangle: index,
            whole: true,
        });
        let piece = Some(Origin {
            triangle: index,
            whole: false,
        });
        let above = s.iter().any(|s| *s > 0);
        let below = s.iter().any(|s| *s < 0);
        match (above, below) {
            (true, false) => {
                upper.triangles.push(t);
                upper.from.push(whole);
            }
            (false, true) => {
                lower.triangles.push(t);
                lower.from.push(whole);
            }
            (false, false) => {
                // A face lying in the plane: the top of what is under it
                // when it faces up, the foot of what is over it when it
                // faces down.
                let [a, b, c] = t.map(|v| points[v as usize]);
                let up = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
                if up < 0.0 {
                    upper.triangles.push(t);
                    upper.from.push(whole);
                } else {
                    lower.triangles.push(t);
                    lower.from.push(whole);
                }
            }
            (true, true) => {
                let mut cut_at = |a: u32, b: u32, points: &mut Vec<[f64; 3]>| -> u32 {
                    let key = if a < b { (a, b) } else { (b, a) };
                    *made.entry(key).or_insert_with(|| {
                        let p = crossing(points[a as usize], points[b as usize], z);
                        points.push(p);
                        u32::try_from(points.len() - 1).expect("mesh exceeds u32 vertex indices")
                    })
                };
                let mut put = |side: i8, tri: [u32; 3]| {
                    let half = if side > 0 { &mut upper } else { &mut lower };
                    half.triangles.push(tri);
                    half.from.push(piece);
                };
                if let Some(k) = (0..3).find(|k| s[*k] == 0) {
                    // One corner on the plane, the other two either side:
                    // the edge between them is cut, and each side takes a
                    // triangle from the corner.
                    let (v0, v1, v2) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let p = cut_at(v1, v2, &mut points);
                    put(s[(k + 1) % 3], [v0, v1, p]);
                    put(s[(k + 2) % 3], [v0, p, v2]);
                } else {
                    // One corner alone on its side: a triangle there, the
                    // rest of the triangle on the other side in two,
                    // across the shorter diagonal.
                    let k = (0..3)
                        .find(|k| s[*k] != s[(*k + 1) % 3] && s[*k] != s[(*k + 2) % 3])
                        .expect("a crossed triangle has a corner alone on its side");
                    let (v0, v1, v2) = (t[k], t[(k + 1) % 3], t[(k + 2) % 3]);
                    let p01 = cut_at(v0, v1, &mut points);
                    let p20 = cut_at(v2, v0, &mut points);
                    put(s[k], [v0, p01, p20]);
                    let other = s[(k + 1) % 3];
                    let length = |a: u32, b: u32| {
                        let d = sub(points[a as usize], points[b as usize]);
                        d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
                    };
                    if length(p01, v2) <= length(v1, p20) {
                        put(other, [p01, v1, v2]);
                        put(other, [p01, v2, p20]);
                    } else {
                        put(other, [v1, p20, p01]);
                        put(other, [v1, v2, p20]);
                    }
                }
            }
        }
    }
    // The cap: over the edges in the plane that the cut opened.
    let on_plane = |v: u32| (v as usize) >= sides.len() || sides[v as usize] == 0;
    let mut runs: FxHashMap<(u32, u32), [i64; 2]> = FxHashMap::default();
    for (which, half) in [&upper, &lower].into_iter().enumerate() {
        for t in &half.triangles {
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if on_plane(a) && on_plane(b) {
                    runs.entry((a, b)).or_default()[which] += 1;
                }
            }
        }
    }
    // An edge the upper half runs from a to b more often than back, and the
    // lower half from b to a: the cut left both open there. Followed so
    // that the lower half's cap runs them a to b.
    let mut edges: Vec<(u32, u32)> = Vec::new();
    let mut keys: Vec<(u32, u32)> = runs.keys().copied().collect();
    keys.sort_unstable();
    for (a, b) in keys {
        let forward = runs[&(a, b)];
        let backward = runs.get(&(b, a)).copied().unwrap_or([0, 0]);
        let upper_open = forward[0] - backward[0];
        let lower_open = backward[1] - forward[1];
        for _ in 0..upper_open.min(lower_open).max(0) {
            edges.push((a, b));
        }
    }
    let (caps, open) = cap(&points, &edges);
    for tri in &caps {
        lower.triangles.push(*tri);
        lower.from.push(None);
        upper.triangles.push([tri[0], tri[2], tri[1]]);
        upper.from.push(None);
    }
    let points = std::sync::Arc::new(points);
    (
        Sided {
            points: points.clone(),
            raw: upper,
        },
        Sided { points, raw: lower },
        open,
    )
}

/// A half over the points it was worked out on.
struct Sided {
    points: std::sync::Arc<Vec<[f64; 3]>>,
    raw: Raw,
}

/// Makes a half's mesh of only the points it uses, taken by `back`.
fn finish(sided: Sided, back: &Transform, outward: bool) -> Half {
    let Sided { points, raw } = sided;
    let mut index: Vec<u32> = vec![u32::MAX; points.len()];
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut triangles = Vec::with_capacity(raw.triangles.len());
    for t in &raw.triangles {
        let mapped = t.map(|v| {
            let slot = &mut index[v as usize];
            if *slot == u32::MAX {
                let p = back.apply(points[v as usize]);
                vertices.push(p.map(|c| c as f32));
                *slot = u32::try_from(vertices.len() - 1).expect("mesh exceeds u32 vertex indices");
            }
            *slot
        });
        triangles.push(if outward {
            [mapped[0], mapped[2], mapped[1]]
        } else {
            mapped
        });
    }
    Half {
        mesh: Mesh {
            vertices,
            triangles,
        },
        from: raw.from,
    }
}

/// Lays caps over the edges given, which run round the lower half's face
/// anticlockwise seen from above (holes clockwise): its triangles, turned
/// to face up, and how many chains did not come round.
fn cap(points: &[[f64; 3]], edges: &[(u32, u32)]) -> (Vec<[u32; 3]>, usize) {
    if edges.is_empty() {
        return (Vec::new(), 0);
    }
    let xy = |v: u32| {
        let p = points[v as usize];
        [p[0], p[1]]
    };
    // The edges out of each corner, in order.
    let mut out: FxHashMap<u32, Vec<usize>> = FxHashMap::default();
    for (at, (a, _)) in edges.iter().enumerate() {
        out.entry(*a).or_default().push(at);
    }
    let mut used = vec![false; edges.len()];
    let mut loops: Vec<Vec<u32>> = Vec::new();
    let mut open = 0;
    for start in 0..edges.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let first = edges[start].0;
        let mut ring = vec![first];
        let (mut from, mut at) = edges[start];
        let closed = loop {
            if at == first {
                break true;
            }
            ring.push(at);
            // Of the edges going on, the one turning furthest left: it keeps
            // to the face this loop goes round where loops touch at a corner.
            let back = {
                let (p, q) = (xy(from), xy(at));
                [p[0] - q[0], p[1] - q[1]]
            };
            let next = out.get(&at).and_then(|list| {
                list.iter().copied().filter(|e| !used[*e]).min_by(|x, y| {
                    let turn = |e: usize| {
                        let (p, q) = (xy(at), xy(edges[e].1));
                        clockwise_from(back, [q[0] - p[0], q[1] - p[1]])
                    };
                    turn(*x).total_cmp(&turn(*y)).then(x.cmp(y))
                })
            });
            let Some(next) = next else { break false };
            used[next] = true;
            from = at;
            at = edges[next].1;
        };
        if closed && ring.len() >= 3 {
            loops.push(ring);
        } else {
            open += 1;
        }
    }
    // Outlines (anticlockwise) and holes (clockwise), each hole in the
    // smallest outline round it.
    let area = |ring: &[u32]| {
        let mut sum = 0.0;
        for k in 0..ring.len() {
            let (p, q) = (xy(ring[k]), xy(ring[(k + 1) % ring.len()]));
            sum += p[0] * q[1] - q[0] * p[1];
        }
        sum / 2.0
    };
    let mut outlines: Vec<(Vec<u32>, f64, Vec<Vec<u32>>)> = Vec::new();
    let mut holes: Vec<Vec<u32>> = Vec::new();
    for ring in loops {
        let a = area(&ring);
        if a > 0.0 {
            outlines.push((ring, a, Vec::new()));
        } else if a < 0.0 {
            holes.push(ring);
        } else {
            open += 1;
        }
    }
    for hole in holes {
        let inside = |outline: &[u32]| {
            // A corner of the hole that is not on the outline, else the
            // middle of one of its edges.
            let probes = hole.iter().map(|v| xy(*v)).chain((0..hole.len()).map(|k| {
                let (p, q) = (xy(hole[k]), xy(hole[(k + 1) % hole.len()]));
                [(p[0] + q[0]) / 2.0, (p[1] + q[1]) / 2.0]
            }));
            for probe in probes {
                match where_is(probe, outline, &xy) {
                    Some(inside) => return inside,
                    None => continue,
                }
            }
            false
        };
        let home = outlines
            .iter()
            .enumerate()
            .filter(|(_, (outline, _, _))| inside(outline))
            .min_by(|x, y| x.1.1.total_cmp(&y.1.1).then(x.0.cmp(&y.0)))
            .map(|(at, _)| at);
        match home {
            Some(at) => outlines[at].2.push(hole),
            None => open += 1,
        }
    }
    let mut triangles = Vec::new();
    for (outline, _, holes) in &outlines {
        let before = triangles.len();
        if !crate::earcut::triangulate(outline, holes, &xy, &mut triangles) {
            triangles.truncate(before);
            open += 1;
        }
    }
    (triangles, open)
}

/// The angle turned clockwise from `from` to `to`, in [0, 2π).
fn clockwise_from(from: [f64; 2], to: [f64; 2]) -> f64 {
    let a = from[1].atan2(from[0]);
    let b = to[1].atan2(to[0]);
    let mut turn = a - b;
    while turn < 0.0 {
        turn += std::f64::consts::TAU;
    }
    while turn >= std::f64::consts::TAU {
        turn -= std::f64::consts::TAU;
    }
    if turn == 0.0 {
        // Straight back the way it came: the last choice.
        std::f64::consts::TAU
    } else {
        turn
    }
}

/// Whether a point is inside a ring (by crossings), or none when it is on
/// the ring's edge.
fn where_is(p: [f64; 2], ring: &[u32], xy: &impl Fn(u32) -> [f64; 2]) -> Option<bool> {
    let mut inside = false;
    for k in 0..ring.len() {
        let (a, b) = (xy(ring[k]), xy(ring[(k + 1) % ring.len()]));
        let cross = (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0]);
        if cross == 0.0
            && p[0] >= a[0].min(b[0])
            && p[0] <= a[0].max(b[0])
            && p[1] >= a[1].min(b[1])
            && p[1] <= a[1].max(b[1])
        {
            return None;
        }
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = a[0] + (p[1] - a[1]) * (b[0] - a[0]) / (b[1] - a[1]);
            if p[0] < x {
                inside = !inside;
            }
        }
    }
    Some(inside)
}
