//! Paint on a mesh's triangles: each triangle split as finely as a stroke
//! needs, each piece with a state (unpainted, an enforcer or a blocker, a
//! filament). A port of the reference's `TriangleSelector`
//! (libslic3r/TriangleSelector.cpp, .hpp), which every painting tool of
//! the C++ application paints with and whose trees a project stores
//! ([`code`]).
//!
//! The tree of each triangle of the mesh is kept as the reference keeps
//! it: pieces in one list, each split one with its children, corners
//! shared between pieces that meet, so that a side split in one triangle
//! is split at the same point in its neighbour. What is left out of the
//! reference: its Spread and Airbrush brushes (EdgeSlicer's own tools),
//! `remap_painting` (carrying paint onto another mesh) and the debug
//! drawing.

pub mod code;
mod cursor;
#[cfg(test)]
mod tests;

use std::sync::Arc;

pub use code::Node;
pub use cursor::{Clip, Cursor, Shape};

use crate::Mesh;

pub type V3 = [f32; 3];

pub(crate) fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
pub(crate) fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn length(a: V3) -> f32 {
    dot(a, a).sqrt()
}
pub(crate) fn normalize(a: V3) -> V3 {
    let l = length(a);
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// An affine map of points, `p' = M p + t`, with `M` given by rows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub m: [[f64; 3]; 3],
    pub t: [f64; 3],
}

impl Affine {
    pub const IDENTITY: Affine = Affine {
        m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        t: [0.0; 3],
    };

    /// From a [`crate::Transform`] (which maps rows: `p' = p · L + t`).
    pub fn from_transform(transform: &crate::Transform) -> Affine {
        let r = &transform.rows;
        Affine {
            m: std::array::from_fn(|i| std::array::from_fn(|j| r[j][i])),
            t: r[3],
        }
    }

    pub fn apply(&self, p: V3) -> V3 {
        std::array::from_fn(|i| {
            (self.m[i][0] as f32) * p[0]
                + (self.m[i][1] as f32) * p[1]
                + (self.m[i][2] as f32) * p[2]
                + self.t[i] as f32
        })
    }

    pub fn apply64(&self, p: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| {
            self.m[i][0] * p[0] + self.m[i][1] * p[1] + self.m[i][2] * p[2] + self.t[i]
        })
    }

    /// How long each axis of the mesh is made (the lengths of the linear
    /// part's columns: `Geometry::Transformation::get_scaling_factor`
    /// for a map without shear).
    pub fn scaling_factors(&self) -> [f64; 3] {
        std::array::from_fn(|j| {
            (0..3)
                .map(|i| self.m[i][j] * self.m[i][j])
                .sum::<f64>()
                .sqrt()
        })
    }

    fn inverse_linear(&self) -> Option<[[f64; 3]; 3]> {
        let m = &self.m;
        let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
            - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
        if det.abs() < 1e-300 {
            return None;
        }
        let c = |a: usize, b: usize, cc: usize, d: usize| m[a][b] * m[cc][d] - m[a][d] * m[cc][b];
        Some([
            [
                c(1, 1, 2, 2) / det,
                -c(0, 1, 2, 2) / det,
                c(0, 1, 1, 2) / det,
            ],
            [
                -c(1, 0, 2, 2) / det,
                c(0, 0, 2, 2) / det,
                -c(0, 0, 1, 2) / det,
            ],
            [
                c(1, 0, 2, 1) / det,
                -c(0, 0, 2, 1) / det,
                c(0, 0, 1, 1) / det,
            ],
        ])
    }

    /// The map undone.
    pub fn inverse(&self) -> Option<Affine> {
        let m = self.inverse_linear()?;
        let t = std::array::from_fn(|i| {
            -(m[i][0] * self.t[0] + m[i][1] * self.t[1] + m[i][2] * self.t[2])
        });
        Some(Affine { m, t })
    }

    /// The inverse transpose of the linear part, by rows, for normals.
    pub fn normal_matrix(&self) -> [V3; 3] {
        let inv = self.inverse_linear().unwrap_or(Affine::IDENTITY.m);
        std::array::from_fn(|i| std::array::from_fn(|j| inv[j][i] as f32))
    }

    /// `self` after `first`.
    pub fn after(&self, first: &Affine) -> Affine {
        let m = std::array::from_fn(|i| {
            std::array::from_fn(|j| (0..3).map(|k| self.m[i][k] * first.m[k][j]).sum())
        });
        let t = std::array::from_fn(|i| {
            (0..3).map(|k| self.m[i][k] * first.t[k]).sum::<f64>() + self.t[i]
        });
        Affine { m, t }
    }
}

/// What does not change while a mesh is painted: each triangle's
/// neighbours and normal. Worked out once for a mesh and shared.
#[derive(Debug)]
pub struct Topology {
    neighbors: Vec<[i32; 3]>,
    normals: Vec<V3>,
}

impl Topology {
    /// `its_face_neighbors` (worked out as `split::neighbours` does, in one
    /// pass) and `its_face_normals`.
    pub fn of(mesh: &Mesh) -> Arc<Topology> {
        let neighbors = crate::split::neighbours(mesh)
            .into_iter()
            .map(|sides| sides.map(|side| side.map_or(-1, |other| other as i32)))
            .collect();
        let normals = (0..mesh.triangles.len())
            .map(|at| {
                let [a, b, c] = mesh.triangle(at);
                normalize(cross(sub(b, a), sub(c, a)))
            })
            .collect();
        Arc::new(Topology { neighbors, normals })
    }
}

#[derive(Clone, Copy, Debug)]
struct Vertex {
    v: V3,
    ref_cnt: i32,
    /// The next free vertex while this one is free (the reference keeps it
    /// in the coordinates).
    next_free: i32,
}

#[derive(Clone, Copy, Debug)]
struct Tri {
    verts: [i32; 3],
    source: i32,
    children: [i32; 4],
    splits: u8,
    special: u8,
    seed: bool,
    valid: bool,
    state: u8,
}

impl Tri {
    fn new(verts: [i32; 3], source: i32, state: u8) -> Tri {
        Tri {
            verts,
            source,
            children: [-1; 4],
            splits: 0,
            special: 0,
            seed: false,
            valid: true,
            state,
        }
    }
    fn is_split(&self) -> bool {
        self.splits != 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    First,
    Second,
}

fn next(i: usize) -> usize {
    (i + 1) % 3
}
fn prev(i: usize) -> usize {
    (i + 2) % 3
}

/// A painted mesh: see the module's own words.
#[derive(Clone)]
pub struct Selector {
    vertices: Vec<Vertex>,
    triangles: Vec<Tri>,
    topology: Arc<Topology>,
    mesh_vertices: Arc<Vec<V3>>,
    mesh_triangles: Arc<Vec<[u32; 3]>>,
    orig_vertices: usize,
    orig_triangles: usize,
    edge_limit_sqr: f32,
    old_cursor_radius_sqr: f32,
    invalid: usize,
    free_triangles: i32,
    free_vertices: i32,
    cursor: Option<Cursor>,
    /// The mesh's triangles whose paint (or whose mark of the last fill)
    /// has changed since [`Selector::take_changed`].
    changed: Vec<u32>,
    changed_mark: Vec<bool>,
}

/// The reference starts every selector with an edge limit of 0.6 mm; a
/// stroke sets it from its cursor's size (`select_patch`).
const START_EDGE_LIMIT: f32 = 0.6;

impl Selector {
    /// A mesh with nothing painted on it (`TriangleSelector::TriangleSelector`).
    pub fn new(mesh: &Mesh, topology: Arc<Topology>) -> Selector {
        assert_eq!(
            topology.neighbors.len(),
            mesh.triangles.len(),
            "the topology is of another mesh"
        );
        let mut out = Selector {
            vertices: Vec::new(),
            triangles: Vec::new(),
            topology,
            mesh_vertices: Arc::new(mesh.vertices.clone()),
            mesh_triangles: Arc::new(mesh.triangles.clone()),
            orig_vertices: 0,
            orig_triangles: 0,
            edge_limit_sqr: START_EDGE_LIMIT * START_EDGE_LIMIT,
            old_cursor_radius_sqr: 0.0,
            invalid: 0,
            free_triangles: -1,
            free_vertices: -1,
            cursor: None,
            changed: Vec::new(),
            changed_mark: vec![false; mesh.triangles.len()],
        };
        out.reset_tree();
        out
    }

    /// How many triangles the mesh has.
    pub fn triangle_count(&self) -> usize {
        self.orig_triangles
    }

    fn reset_tree(&mut self) {
        self.vertices.clear();
        self.triangles.clear();
        self.invalid = 0;
        self.free_triangles = -1;
        self.free_vertices = -1;
        self.vertices
            .extend(self.mesh_vertices.iter().map(|&v| Vertex {
                v,
                ref_cnt: 0,
                next_free: -1,
            }));
        let mesh_triangles = Arc::clone(&self.mesh_triangles);
        self.triangles.reserve(mesh_triangles.len());
        for (at, t) in mesh_triangles.iter().enumerate() {
            self.push_triangle(t.map(|v| v as i32), at as i32, 0);
        }
        self.orig_vertices = self.vertices.len();
        self.orig_triangles = self.triangles.len();
    }

    /// Takes all paint away (`reset`).
    pub fn reset(&mut self) {
        for at in 0..self.orig_triangles {
            let t = &self.triangles[at];
            if t.is_split() || t.state != 0 || t.seed {
                self.mark(at);
            }
        }
        self.reset_tree();
    }

    fn mark(&mut self, source: usize) {
        if !self.changed_mark[source] {
            self.changed_mark[source] = true;
            self.changed.push(source as u32);
        }
    }

    /// The mesh's triangles whose paint has changed since this was last
    /// asked, in ascending order.
    pub fn take_changed(&mut self) -> Vec<u32> {
        let mut out = std::mem::take(&mut self.changed);
        for &at in &out {
            self.changed_mark[at as usize] = false;
        }
        out.sort_unstable();
        out
    }

    // ---- the tree ----

    fn push_triangle(&mut self, verts: [i32; 3], source: i32, state: u8) -> i32 {
        for v in verts {
            self.vertices[v as usize].ref_cnt += 1;
        }
        if self.free_triangles == -1 {
            self.triangles.push(Tri::new(verts, source, state));
            (self.triangles.len() - 1) as i32
        } else {
            let at = self.free_triangles;
            self.free_triangles = self.triangles[at as usize].children[0];
            self.invalid -= 1;
            self.triangles[at as usize] = Tri::new(verts, source, state);
            at
        }
    }

    fn set_division(&mut self, at: i32, sides: u8, special: u8) {
        let t = &mut self.triangles[at as usize];
        t.splits = sides;
        t.special = special;
    }

    /// The child of `t` touching its side from `vi` to `vj`, the first or
    /// the second part of it; -1 where that side is not split
    /// (`neighbor_child`).
    fn neighbor_child_of(&self, t: &Tri, vi: i32, vj: i32, part: Part) -> i32 {
        if t.splits == 0 {
            return -1;
        }
        let edge = if t.verts[0] == vi {
            0
        } else if t.verts[1] == vi {
            1
        } else {
            2
        };
        debug_assert_eq!(t.verts[next(edge)], vj);
        let special = usize::from(t.special);
        let child = match t.splits {
            1 => {
                if edge != next(special) {
                    let c = t.children[if edge == special { 0 } else { 1 }];
                    return self.neighbor_child_of(&self.triangles[c as usize], vi, vj, part);
                }
                if part == Part::First { 0 } else { 1 }
            }
            2 => {
                if edge == next(special) {
                    return self.neighbor_child_of(
                        &self.triangles[t.children[2] as usize],
                        vi,
                        vj,
                        part,
                    );
                }
                if edge == special {
                    if part == Part::First { 0 } else { 1 }
                } else if part == Part::First {
                    2
                } else {
                    0
                }
            }
            _ => match edge {
                0 => {
                    if part == Part::First {
                        0
                    } else {
                        1
                    }
                }
                1 => {
                    if part == Part::First {
                        1
                    } else {
                        2
                    }
                }
                _ => {
                    if part == Part::First {
                        2
                    } else {
                        0
                    }
                }
            },
        };
        t.children[child]
    }

    fn neighbor_child(&self, at: i32, vi: i32, vj: i32, part: Part) -> i32 {
        if at == -1 {
            -1
        } else {
            self.neighbor_child_of(&self.triangles[at as usize], vi, vj, part)
        }
    }

    /// The children of `t` touching its side from `vi` to `vj`
    /// (`triangle_subtriangles`).
    fn subtriangles_of(t: &Tri, vi: i32, vj: i32) -> (i32, i32) {
        if t.splits == 0 {
            return (-1, -1);
        }
        let edge = if t.verts[0] == vi {
            0
        } else if t.verts[1] == vi {
            1
        } else {
            2
        };
        debug_assert_eq!(t.verts[next(edge)], vj);
        let special = usize::from(t.special);
        let c = &t.children;
        match t.splits {
            1 => {
                if edge == next(special) {
                    (c[0], c[1])
                } else {
                    (c[if edge == special { 0 } else { 1 }], -1)
                }
            }
            2 => {
                if edge == next(special) {
                    (c[2], -1)
                } else if edge == special {
                    (c[0], c[1])
                } else {
                    (c[2], c[0])
                }
            }
            _ => match edge {
                0 => (c[0], c[1]),
                1 => (c[1], c[2]),
                _ => (c[2], c[0]),
            },
        }
    }

    fn subtriangles(&self, at: i32, vi: i32, vj: i32) -> (i32, i32) {
        if at == -1 {
            (-1, -1)
        } else {
            Self::subtriangles_of(&self.triangles[at as usize], vi, vj)
        }
    }

    /// The corner at the middle of `t`'s side from `vi` to `vj`, where
    /// that side is split; -1 where it is not (`triangle_midpoint`).
    fn midpoint_of(&self, t: &Tri, vi: i32, vj: i32) -> i32 {
        if t.splits == 0 {
            return -1;
        }
        let edge = if t.verts[0] == vi {
            0
        } else if t.verts[1] == vi {
            1
        } else {
            2
        };
        debug_assert_eq!(t.verts[next(edge)], vj);
        let special = usize::from(t.special);
        let child = |k: usize| &self.triangles[t.children[k] as usize];
        match t.splits {
            1 => {
                if edge == next(special) {
                    child(0).verts[2]
                } else {
                    self.midpoint_of(child(if edge == special { 0 } else { 1 }), vi, vj)
                }
            }
            2 => {
                if edge == next(special) {
                    self.midpoint_of(child(2), vi, vj)
                } else if edge == special {
                    child(0).verts[1]
                } else {
                    child(1).verts[2]
                }
            }
            _ => match edge {
                0 => child(0).verts[1],
                1 => child(1).verts[2],
                _ => child(2).verts[2],
            },
        }
    }

    fn midpoint(&self, at: i32, vi: i32, vj: i32) -> i32 {
        if at == -1 {
            -1
        } else {
            self.midpoint_of(&self.triangles[at as usize], vi, vj)
        }
    }

    fn midpoint_or_allocate(&mut self, at: i32, vi: i32, vj: i32) -> i32 {
        let found = self.midpoint(at, vi, vj);
        if found != -1 {
            return found;
        }
        let (a, b) = (self.vertices[vi as usize].v, self.vertices[vj as usize].v);
        let c = scale(add(a, b), 0.5);
        if self.free_vertices == -1 {
            self.vertices.push(Vertex {
                v: c,
                ref_cnt: 0,
                next_free: -1,
            });
            (self.vertices.len() - 1) as i32
        } else {
            let at = self.free_vertices;
            self.free_vertices = self.vertices[at as usize].next_free;
            self.vertices[at as usize] = Vertex {
                v: c,
                ref_cnt: 0,
                next_free: -1,
            };
            at
        }
    }

    /// The neighbours of child `k` of `t`, given `t`'s (`child_neighbors`).
    fn child_neighbors(&self, t: &Tri, n: [i32; 3], k: usize) -> [i32; 3] {
        let i = usize::from(t.special);
        let j = next(i);
        let kk = next(j);
        let v = &t.verts;
        let c = &t.children;
        match t.splits {
            1 => match k {
                0 => [
                    n[i],
                    self.neighbor_child(n[j], v[kk], v[j], Part::Second),
                    c[1],
                ],
                _ => [
                    self.neighbor_child(n[j], v[kk], v[j], Part::First),
                    n[kk],
                    c[0],
                ],
            },
            2 => match k {
                0 => [
                    self.neighbor_child(n[i], v[j], v[i], Part::Second),
                    c[1],
                    self.neighbor_child(n[kk], v[i], v[kk], Part::First),
                ],
                1 => [
                    self.neighbor_child(n[i], v[j], v[i], Part::First),
                    c[2],
                    c[0],
                ],
                _ => [
                    n[j],
                    self.neighbor_child(n[kk], v[i], v[kk], Part::Second),
                    c[1],
                ],
            },
            _ => match k {
                0 => [
                    self.neighbor_child(n[0], v[1], v[0], Part::Second),
                    c[3],
                    self.neighbor_child(n[2], v[0], v[2], Part::First),
                ],
                1 => [
                    self.neighbor_child(n[0], v[1], v[0], Part::First),
                    self.neighbor_child(n[1], v[2], v[1], Part::Second),
                    c[3],
                ],
                2 => [
                    self.neighbor_child(n[1], v[2], v[1], Part::First),
                    self.neighbor_child(n[2], v[0], v[2], Part::Second),
                    c[3],
                ],
                _ => [c[1], c[2], c[0]],
            },
        }
    }

    /// `child_neighbors_propagated`: where a child has no neighbour of its
    /// own depth, its parent's.
    fn child_neighbors_propagated(
        t: &Tri,
        propagated: [i32; 3],
        k: usize,
        child: [i32; 3],
    ) -> [i32; 3] {
        let i = usize::from(t.special);
        let j = next(i);
        let kk = next(j);
        let mut out = child;
        let mut fill = |slot: usize, from: usize| {
            if out[slot] == -1 {
                out[slot] = propagated[from];
            }
        };
        match t.splits {
            1 => match k {
                0 => {
                    fill(0, i);
                    fill(1, j);
                }
                _ => {
                    fill(0, j);
                    fill(1, kk);
                }
            },
            2 => match k {
                0 => {
                    fill(0, i);
                    fill(2, kk);
                }
                1 => fill(0, i),
                _ => {
                    fill(0, j);
                    fill(1, kk);
                }
            },
            _ => match k {
                0 => {
                    fill(0, 0);
                    fill(2, 2);
                }
                1 => {
                    fill(0, 0);
                    fill(1, 1);
                }
                2 => {
                    fill(0, 1);
                    fill(1, 2);
                }
                _ => {}
            },
        }
        out
    }

    /// Makes the children of a triangle whose division is set
    /// (`perform_split`).
    fn perform_split(&mut self, at: i32, n: [i32; 3], old_state: u8) {
        let t = self.triangles[at as usize];
        let special = usize::from(t.special);
        let mut v: Vec<i32> = (0..3).map(|k| t.verts[(special + k) % 3]).collect();
        let source = t.source;
        let mut children = [-1; 4];
        match t.splits {
            1 => {
                let m = self.midpoint_or_allocate(n[next(special)], v[2], v[1]);
                v.insert(2, m);
                children[0] = self.push_triangle([v[0], v[1], v[2]], source, old_state);
                children[1] = self.push_triangle([v[2], v[3], v[0]], source, old_state);
            }
            2 => {
                let m = self.midpoint_or_allocate(n[special], v[1], v[0]);
                v.insert(1, m);
                let m = self.midpoint_or_allocate(n[prev(special)], v[0], v[3]);
                v.insert(4, m);
                children[0] = self.push_triangle([v[0], v[1], v[4]], source, old_state);
                children[1] = self.push_triangle([v[1], v[2], v[4]], source, old_state);
                children[2] = self.push_triangle([v[2], v[3], v[4]], source, old_state);
            }
            _ => {
                let m = self.midpoint_or_allocate(n[0], v[1], v[0]);
                v.insert(1, m);
                let m = self.midpoint_or_allocate(n[1], v[3], v[2]);
                v.insert(3, m);
                let m = self.midpoint_or_allocate(n[2], v[0], v[4]);
                v.insert(5, m);
                children[0] = self.push_triangle([v[0], v[1], v[5]], source, old_state);
                children[1] = self.push_triangle([v[1], v[2], v[3]], source, old_state);
                children[2] = self.push_triangle([v[3], v[4], v[5]], source, old_state);
                children[3] = self.push_triangle([v[1], v[3], v[5]], source, old_state);
            }
        }
        self.triangles[at as usize].children = children;
    }

    /// Takes away all the children of a triangle (`undivide_triangle`).
    fn undivide(&mut self, at: i32) {
        let t = self.triangles[at as usize];
        if !t.is_split() {
            return;
        }
        for k in 0..=usize::from(t.splits) {
            let child = t.children[k];
            self.undivide(child);
            let verts = self.triangles[child as usize].verts;
            for v in verts {
                let vertex = &mut self.vertices[v as usize];
                vertex.ref_cnt -= 1;
                if vertex.ref_cnt == 0 {
                    vertex.next_free = self.free_vertices;
                    self.free_vertices = v;
                }
            }
            let c = &mut self.triangles[child as usize];
            c.valid = false;
            c.children[0] = self.free_triangles;
            self.free_triangles = child;
            self.invalid += 1;
        }
        self.set_division(at, 0, 0);
    }

    /// Where all the children of a triangle are pieces of one state, puts
    /// the triangle back whole in that state (`remove_useless_children`).
    fn remove_useless_children(&mut self, at: i32) {
        let t = self.triangles[at as usize];
        if !t.is_split() {
            return;
        }
        for k in 0..=usize::from(t.splits) {
            if self.triangles[t.children[k] as usize].is_split() {
                self.remove_useless_children(t.children[k]);
            }
        }
        let mut first = 0;
        for k in 0..=usize::from(t.splits) {
            let c = &self.triangles[t.children[k] as usize];
            if c.is_split() {
                return;
            }
            if k == 0 {
                first = c.state;
            } else if c.state != first {
                return;
            }
        }
        self.undivide(at);
        self.triangles[at as usize].state = first;
    }

    /// Drops the pieces and corners no longer used (`garbage_collect`).
    fn garbage_collect(&mut self) {
        let mut new_triangles = vec![-1i32; self.triangles.len()];
        let mut next_at = self.orig_triangles as i32;
        for (at, slot) in new_triangles
            .iter_mut()
            .enumerate()
            .skip(self.orig_triangles)
        {
            if self.triangles[at].valid {
                *slot = next_at;
                next_at += 1;
            }
        }
        let mut new_vertices = vec![-1i32; self.vertices.len()];
        let mut next_v = self.orig_vertices as i32;
        for (at, slot) in new_vertices.iter_mut().enumerate().skip(self.orig_vertices) {
            if self.vertices[at].ref_cnt != 0 {
                *slot = next_v;
                next_v += 1;
            }
        }
        let orig_t = self.orig_triangles;
        let mut index = 0;
        self.triangles.retain(|t| {
            let keep = index < orig_t || t.valid;
            index += 1;
            keep
        });
        let orig_v = self.orig_vertices;
        let mut index = 0;
        self.vertices.retain(|v| {
            let keep = index < orig_v || v.ref_cnt != 0;
            index += 1;
            keep
        });
        for t in &mut self.triangles {
            if t.is_split() {
                for k in 0..=usize::from(t.splits) {
                    t.children[k] = new_triangles[t.children[k] as usize];
                }
            }
            for v in &mut t.verts {
                if *v as usize >= orig_v {
                    *v = new_vertices[*v as usize];
                }
            }
        }
        self.invalid = 0;
        self.free_triangles = -1;
        self.free_vertices = -1;
    }

    fn corners(&self, t: &Tri) -> [V3; 3] {
        t.verts.map(|v| self.vertices[v as usize].v)
    }

    // ---- painting with a cursor ----

    /// Sets the longest side a stroke leaves a piece with, in millimetres
    /// of the mesh (`set_edge_limit`).
    pub fn set_edge_limit(&mut self, limit: f32) {
        self.edge_limit_sqr = limit * limit;
    }

    /// Splits a triangle on its sides longer than the edge limit
    /// (`split_triangle`).
    fn split_triangle(&mut self, at: i32, n: [i32; 3]) {
        if self.triangles[at as usize].is_split() {
            return;
        }
        let t = self.triangles[at as usize];
        let old = t.state;
        let limit = f64::from(self.edge_limit_sqr);
        let mut pts = self.corners(&t);
        let cursor = self.cursor.as_ref().expect("a stroke has a cursor");
        if !cursor.uniform {
            pts = pts.map(|p| cursor.trafo.apply(p));
        }
        let sq = |a: V3, b: V3| f64::from(dot(sub(a, b), sub(a, b)));
        let sides = [sq(pts[2], pts[1]), sq(pts[0], pts[2]), sq(pts[1], pts[0])];
        let mut split: Vec<u8> = Vec::with_capacity(3);
        let mut keep = 0u8;
        for (k, side) in sides.iter().enumerate() {
            if *side > limit {
                split.push(k as u8);
            } else {
                keep = k as u8;
            }
        }
        if split.is_empty() {
            self.set_division(at, 0, 0);
            return;
        }
        let special = if split.len() == 2 { keep } else { split[0] };
        let special = if split.len() == 3 { 0 } else { special };
        self.set_division(at, split.len() as u8, special);
        self.perform_split(at, n, old);
    }

    fn select_triangle_recursive(
        &mut self,
        at: i32,
        n: [i32; 3],
        state: u8,
        splitting: bool,
        partially: bool,
    ) -> bool {
        let t = self.triangles[at as usize];
        if !t.valid {
            return false;
        }
        let corners = self.corners(&t);
        let cursor = self.cursor.as_ref().expect("a stroke has a cursor");
        let inside = corners
            .iter()
            .filter(|&&corner| cursor.point_inside(corner))
            .count();
        if inside == 0 && !cursor.pointer_in(corners) && !cursor.edge_inside(corners) {
            return false;
        }
        if inside == 3 {
            self.undivide(at);
            self.triangles[at as usize].state = state;
            return true;
        }
        if !t.is_split() && t.state == state {
            return true;
        }
        if splitting {
            self.split_triangle(at, n);
        }
        if (!splitting || partially) && !self.triangles[at as usize].is_split() {
            self.triangles[at as usize].state = state;
            return true;
        }
        let t = self.triangles[at as usize];
        if t.is_split() {
            for k in 0..=usize::from(t.splits) {
                let t = self.triangles[at as usize];
                let child_n = self.child_neighbors(&t, n, k);
                self.select_triangle_recursive(t.children[k], child_n, state, splitting, partially);
            }
        }
        true
    }

    /// Paints what of a triangle of the mesh is under the cursor
    /// (`select_triangle`); false when none of it is.
    fn select_triangle(&mut self, at: usize, state: u8, splitting: bool, partially: bool) -> bool {
        if !self.triangles[at].valid {
            return false;
        }
        let n = self.topology.neighbors[at];
        if !self.select_triangle_recursive(at as i32, n, state, splitting, partially) {
            return false;
        }
        self.mark(at);
        self.remove_useless_children(at as i32);
        if 2 * self.invalid > self.triangles.len() {
            self.garbage_collect();
        }
        true
    }

    /// Paints `state` on what the cursor takes in, starting from triangle
    /// `start` of the mesh, which the pointer is on, and spreading to the
    /// neighbours it reaches (`select_patch`). `world` is the map from the
    /// mesh to the world; with `highlight_deg` above zero only triangles
    /// facing down more steeply than that are painted. With
    /// `splitting` triangles are cut to follow the cursor's edge; with
    /// `partially` a triangle too small to cut is painted whole.
    #[allow(clippy::too_many_arguments)]
    pub fn select_patch(
        &mut self,
        start: usize,
        cursor: Cursor,
        state: u8,
        world: &Affine,
        splitting: bool,
        highlight_deg: f32,
        partially: bool,
    ) {
        let height_range = matches!(cursor.shape, Shape::HeightRange { .. });
        if self.old_cursor_radius_sqr != cursor.radius_sqr {
            if height_range {
                self.set_edge_limit(0.1);
                self.old_cursor_radius_sqr = 0.1;
            } else {
                self.set_edge_limit((cursor.radius_sqr.sqrt() / 5.0).min(0.05));
                self.old_cursor_radius_sqr = cursor.radius_sqr;
            }
        }
        self.cursor = Some(cursor);
        let limit = -(f64::from(highlight_deg).to_radians().cos()) as f32;
        let normal_matrix = world.normal_matrix();
        let starts: Vec<usize> = if height_range {
            let cursor = self.cursor.as_ref().expect("set above");
            (0..self.orig_triangles)
                .filter(|&at| cursor.edge_inside(self.corners(&self.triangles[at])))
                .collect()
        } else {
            vec![start]
        };
        let mut visited = vec![false; self.orig_triangles];
        let mut queue: Vec<usize> = Vec::with_capacity(16);
        for first in starts {
            if visited[first] {
                continue;
            }
            queue.clear();
            queue.push(first);
            let mut head = 0;
            while head < queue.len() {
                let facet = queue[head];
                let normal = self.topology.normals[self.triangles[facet].source as usize];
                let world_z = normalize([
                    dot(normal_matrix[0], normal),
                    dot(normal_matrix[1], normal),
                    dot(normal_matrix[2], normal),
                ])[2];
                if !visited[facet]
                    && (highlight_deg == 0.0 || world_z < limit)
                    && self.select_triangle(facet, state, splitting, partially)
                {
                    let cursor = self.cursor.as_ref().expect("set above");
                    for neighbor in self.topology.neighbors[facet] {
                        if neighbor >= 0
                            && cursor.facet_visible(self.topology.normals[neighbor as usize])
                        {
                            queue.push(neighbor as usize);
                        }
                    }
                }
                visited[facet] = true;
                head += 1;
            }
        }
    }

    /// Paints a triangle of the mesh whole (`set_facet`).
    pub fn set_facet(&mut self, at: usize, state: u8) {
        self.undivide(at as i32);
        self.triangles[at].state = state;
        self.mark(at);
    }

    // ---- filling ----

    /// The piece of the mesh's triangle `facet` a point is on, or -1
    /// (`select_unsplit_triangle`).
    fn unsplit_at(&self, hit: V3, facet: usize) -> i32 {
        if !self.triangles[facet].valid {
            return -1;
        }
        self.unsplit_in(hit, facet as i32, self.topology.neighbors[facet])
    }

    fn unsplit_in(&self, hit: V3, at: i32, n: [i32; 3]) -> i32 {
        let t = &self.triangles[at as usize];
        if !t.valid {
            return -1;
        }
        if !t.is_split() {
            return if point_in_triangle(hit, self.corners(t)) {
                at
            } else {
                -1
            };
        }
        for k in 0..=usize::from(t.splits) {
            let c = &self.triangles[t.children[k] as usize];
            if point_in_triangle(hit, self.corners(c)) {
                return self.unsplit_in(hit, t.children[k], self.child_neighbors(t, n, k));
            }
        }
        -1
    }

    fn facet_clipped(&self, at: usize, clip: Option<Clip>) -> bool {
        clip.is_some_and(|clip| {
            self.triangles[at]
                .verts
                .iter()
                .any(|&v| clip.clipped(self.vertices[v as usize].v))
        })
    }

    /// Takes away every mark of a fill (`seed_fill_unselect_all_triangles`).
    pub fn unmark_fill(&mut self) {
        for at in 0..self.triangles.len() {
            let t = self.triangles[at];
            if t.valid && !t.is_split() && t.seed {
                self.triangles[at].seed = false;
                self.mark(t.source as usize);
            }
        }
    }

    /// Paints `state` on every piece a fill marked, and puts back whole
    /// what that leaves alike (`seed_fill_apply_on_triangles`).
    pub fn apply_fill(&mut self, state: u8) {
        for at in 0..self.triangles.len() {
            let t = self.triangles[at];
            if t.valid && !t.is_split() && t.seed {
                self.triangles[at].state = state;
                self.mark(t.source as usize);
            }
        }
        for at in 0..self.triangles.len() {
            let t = self.triangles[at];
            if t.is_split() && t.valid {
                self.remove_useless_children(at as i32);
            }
        }
    }

    /// Whether any piece is marked by a fill.
    pub fn has_fill(&self) -> bool {
        self.triangles
            .iter()
            .any(|t| t.valid && !t.is_split() && t.seed)
    }

    /// "Fill" (smart fill): marks the triangles reached from `start` across
    /// edges where the faces turn by no more than `angle_deg`
    /// (`seed_fill_select_triangles`). Nothing is marked anew while the
    /// pointer stays on what is marked, unless `force`.
    #[allow(clippy::too_many_arguments)]
    pub fn seed_fill(
        &mut self,
        hit: V3,
        start: usize,
        world: &Affine,
        clip: Option<Clip>,
        angle_deg: f32,
        highlight_deg: f32,
        force: bool,
    ) {
        let on = self.unsplit_at(hit, start);
        if on >= 0 && self.triangles[on as usize].seed && !force && clip.is_none() {
            return;
        }
        self.unmark_fill();
        let mut visited = vec![false; self.triangles.len()];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(start);
        let facet_limit = f64::from(angle_deg).to_radians().cos() - 1e-4;
        let limit = -(f64::from(highlight_deg).to_radians().cos()) as f32;
        let normal_matrix = world.normal_matrix();
        while let Some(current) = queue.pop_front() {
            let normal = self.topology.normals[self.triangles[current].source as usize];
            let world_z = normalize([
                dot(normal_matrix[0], normal),
                dot(normal_matrix[1], normal),
                dot(normal_matrix[2], normal),
            ])[2];
            if !visited[current] && (highlight_deg == 0.0 || world_z < limit) {
                let t = self.triangles[current];
                if t.is_split() {
                    for k in 0..=usize::from(t.splits) {
                        let child = t.children[k] as usize;
                        if !visited[child] {
                            queue.push_back(child);
                        }
                    }
                } else {
                    self.triangles[current].seed = true;
                    self.mark(t.source as usize);
                }
                if current < self.orig_triangles {
                    for neighbor in self.topology.neighbors[current] {
                        if neighbor >= 0
                            && !visited[neighbor as usize]
                            && !self.facet_clipped(neighbor as usize, clip)
                        {
                            let n1 = self.topology.normals
                                [self.triangles[neighbor as usize].source as usize];
                            let n2 = self.topology.normals[self.triangles[current].source as usize];
                            if f64::from(dot(n1, n2).clamp(0.0, 1.0)) >= facet_limit {
                                queue.push_back(neighbor as usize);
                            }
                        }
                    }
                }
            }
            visited[current] = true;
        }
    }

    fn all_neighbors_recursive(
        &self,
        at: i32,
        n: [i32; 3],
        propagated: [i32; 3],
        out_n: &mut [[i32; 3]],
        out_p: &mut [[i32; 3]],
    ) {
        let t = &self.triangles[at as usize];
        if !t.valid {
            return;
        }
        out_n[at as usize] = n;
        out_p[at as usize] = propagated;
        if t.is_split() {
            for k in 0..=usize::from(t.splits) {
                let child = self.child_neighbors(t, n, k);
                let child_p = Self::child_neighbors_propagated(t, propagated, k, child);
                self.all_neighbors_recursive(t.children[k], child, child_p, out_n, out_p);
            }
        }
    }

    /// Every piece's neighbours, and where it has none of its own depth its
    /// parent's (`precompute_all_neighbors`).
    fn all_neighbors(&self) -> (Vec<[i32; 3]>, Vec<[i32; 3]>) {
        let mut n = vec![[-1; 3]; self.triangles.len()];
        let mut p = vec![[-1; 3]; self.triangles.len()];
        for at in 0..self.orig_triangles {
            n[at] = self.topology.neighbors[at];
            p[at] = n[at];
            if self.triangles[at].is_split() {
                let first = n[at];
                self.all_neighbors_recursive(at as i32, first, first, &mut n, &mut p);
            }
        }
        (n, p)
    }

    fn touching_subtriangles(&self, at: i32, vi: i32, vj: i32, out: &mut Vec<i32>) {
        if at == -1 {
            return;
        }
        let (first, second) = self.subtriangles(at, vi, vj);
        for (sub, part) in [(first, Part::First), (second, Part::Second)] {
            if sub == -1 {
                continue;
            }
            if !self.triangles[sub as usize].is_split() {
                out.push(sub);
            } else {
                let m = self.midpoint(at, vi, vj);
                if m != -1 {
                    if part == Part::First {
                        self.touching_subtriangles(sub, vi, m, out);
                    } else {
                        self.touching_subtriangles(sub, m, vj, out);
                    }
                } else {
                    self.touching_subtriangles(sub, vi, vj, out);
                }
            }
        }
    }

    /// The pieces touching piece `at` (`get_all_touching_triangles`).
    fn touching(&self, at: usize, n: [i32; 3], propagated: [i32; 3]) -> Vec<i32> {
        let v = self.triangles[at].verts;
        let mut out = Vec::new();
        self.touching_subtriangles(n[0], v[1], v[0], &mut out);
        self.touching_subtriangles(n[1], v[2], v[1], &mut out);
        self.touching_subtriangles(n[2], v[0], v[2], &mut out);
        for neighbor in propagated {
            if neighbor != -1 && !self.triangles[neighbor as usize].is_split() {
                out.push(neighbor);
            }
        }
        out
    }

    /// The colour gizmo's "Fill" (`propagate`) or "Triangle": marks the
    /// pieces of the same state reached from the one the pointer is on
    /// across edges where the faces turn by no more than `angle_deg` (a
    /// negative angle: any turn), or only that piece
    /// (`bucket_fill_select_triangles`).
    pub fn bucket_fill(
        &mut self,
        hit: V3,
        start: usize,
        clip: Option<Clip>,
        angle_deg: f32,
        propagate: bool,
        force: bool,
    ) {
        let first = self.unsplit_at(hit, start);
        if first == -1 || (self.triangles[first as usize].seed && !force && clip.is_none()) {
            return;
        }
        let first = first as usize;
        let state = self.triangles[first].state;
        self.unmark_fill();
        if !propagate {
            self.triangles[first].seed = true;
            self.mark(self.triangles[first].source as usize);
            return;
        }
        let facet_limit = if angle_deg < 0.0 {
            -1.0
        } else {
            f64::from(angle_deg).to_radians().cos()
        } - 1e-4;
        let (n, p) = self.all_neighbors();
        let mut visited = vec![false; self.triangles.len()];
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(first);
        while let Some(current) = queue.pop_front() {
            if !visited[current] {
                self.triangles[current].seed = true;
                self.mark(self.triangles[current].source as usize);
                for other in self.touching(current, n[current], p[current]) {
                    if other < 0 {
                        continue;
                    }
                    let other = other as usize;
                    if visited[other]
                        || self.triangles[other].state != state
                        || self.facet_clipped(other, clip)
                    {
                        continue;
                    }
                    let n1 = self.topology.normals[self.triangles[other].source as usize];
                    let n2 = self.topology.normals[self.triangles[current].source as usize];
                    if f64::from(angle_deg) >= -1e-4
                        && f64::from(dot(n1, n2).clamp(0.0, 1.0)) < facet_limit
                    {
                        continue;
                    }
                    queue.push_back(other);
                }
            }
            visited[current] = true;
        }
    }

    // ---- gap fill ----

    /// The pieces in groups of one state joined across their sides, each
    /// with its area (summed up to `GapAreaMax`, 5 mm², where the reference
    /// stops counting) and the least state of the pieces round it
    /// (`TriangleSelectorPatch::update_triangles_per_patch`).
    fn patches(&self) -> Vec<(Vec<usize>, f32, Option<u8>)> {
        const GAP_AREA_MAX: f64 = 5.0;
        let (n, p) = self.all_neighbors();
        let mut visited = vec![false; self.triangles.len()];
        let mut out = Vec::new();
        let mut start = 0;
        loop {
            while start < visited.len()
                && (visited[start]
                    || !self.triangles[start].valid
                    || self.triangles[start].is_split())
            {
                start += 1;
            }
            if start >= self.triangles.len() {
                break;
            }
            let state = self.triangles[start].state;
            let mut pieces = Vec::new();
            let mut around: Option<u8> = None;
            let mut queue = std::collections::VecDeque::new();
            queue.push_back(start);
            while let Some(current) = queue.pop_front() {
                if !visited[current] {
                    pieces.push(current);
                    for other in self.touching(current, n[current], p[current]) {
                        if other < 0 {
                            continue;
                        }
                        let other = other as usize;
                        let other_state = self.triangles[other].state;
                        if other_state != state {
                            around =
                                Some(around.map_or(other_state, |least| least.min(other_state)));
                            continue;
                        }
                        if !visited[other] {
                            queue.push_back(other);
                        }
                    }
                }
                visited[current] = true;
            }
            let mut area = 0.0f64;
            for &piece in &pieces {
                let [a, b, c] = self.corners(&self.triangles[piece]);
                area += f64::from(length(cross(sub(a, b), sub(a, c))) / 2.0);
                if area >= GAP_AREA_MAX {
                    break;
                }
            }
            out.push((pieces, area as f32, around));
        }
        out
    }

    /// What "Perform" of "Gap fill" would paint: each piece of a group
    /// smaller than `gap_area` mm², with the least state round it. A piece
    /// listed once.
    pub fn gaps(&self, gap_area: f32) -> Vec<(usize, u8)> {
        let mut out = Vec::new();
        for (pieces, area, around) in self.patches() {
            if area < gap_area
                && let Some(state) = around
            {
                out.extend(pieces.into_iter().map(|piece| (piece, state)));
            }
        }
        out
    }

    /// "Perform" (`update_selector_triangles`): each gap takes the state
    /// round it.
    pub fn fill_gaps(&mut self, gap_area: f32) {
        for (piece, state) in self.gaps(gap_area) {
            self.triangles[piece].state = state;
            self.mark(self.triangles[piece].source as usize);
        }
    }

    // ---- what there is ----

    /// The pieces of the mesh's triangle `at`, each with its corners, its
    /// state and whether a fill marks it, in the order the tree holds them.
    pub fn leaves(&self, at: usize, out: &mut Vec<([V3; 3], u8, bool)>) {
        self.leaves_of(at as i32, out);
    }

    fn leaves_of(&self, at: i32, out: &mut Vec<([V3; 3], u8, bool)>) {
        let t = &self.triangles[at as usize];
        if t.is_split() {
            for k in 0..=usize::from(t.splits) {
                self.leaves_of(t.children[k], out);
            }
        } else {
            out.push((self.corners(t), t.state, t.seed));
        }
    }

    /// The piece index of each piece of triangle `at` (for [`Selector::gaps`]).
    pub fn leaf_ids(&self, at: usize, out: &mut Vec<usize>) {
        let t = &self.triangles[at];
        if t.is_split() {
            for k in 0..=usize::from(t.splits) {
                self.leaf_ids(t.children[k] as usize, out);
            }
        } else {
            out.push(at);
        }
    }

    /// The mesh's triangle a piece is of.
    pub fn source_of(&self, piece: usize) -> usize {
        self.triangles[piece].source as usize
    }

    /// Whether anything is painted with `state` (`has_facets`).
    pub fn has_state(&self, state: u8) -> bool {
        self.triangles
            .iter()
            .any(|t| t.valid && !t.is_split() && t.state == state)
    }

    /// Whether anything is painted at all.
    pub fn is_painted(&self) -> bool {
        (0..self.orig_triangles)
            .any(|at| self.triangles[at].is_split() || self.triangles[at].state != 0)
    }

    /// The states painted, each once, in ascending order, unpainted left
    /// out.
    pub fn states(&self) -> Vec<u8> {
        let mut seen = [false; 256];
        for t in &self.triangles {
            if t.valid && !t.is_split() {
                seen[usize::from(t.state)] = true;
            }
        }
        (1..256).filter(|&s| seen[s]).map(|s| s as u8).collect()
    }

    /// The sides round what a fill marks, as pairs of corners
    /// (`get_seed_fill_contour`).
    pub fn fill_contour(&self) -> Vec<[V3; 2]> {
        let mut edges: Vec<[i32; 2]> = Vec::new();
        for at in 0..self.orig_triangles {
            let n = self.topology.neighbors[at];
            self.contour_recursive(at as i32, n, n, &mut edges);
        }
        edges
            .into_iter()
            .map(|[a, b]| [self.vertices[a as usize].v, self.vertices[b as usize].v])
            .collect()
    }

    fn touching_edges(&self, at: i32, vi: i32, vj: i32, out: &mut Vec<[i32; 2]>) {
        if at == -1 {
            return;
        }
        let (first, second) = self.subtriangles(at, vi, vj);
        for (sub, part) in [(first, Part::First), (second, Part::Second)] {
            if sub == -1 {
                continue;
            }
            let m = self.midpoint(at, vi, vj);
            if !self.triangles[sub as usize].is_split() {
                if !self.triangles[sub as usize].seed {
                    out.push(match (part, m) {
                        (Part::First, -1) => [vi, vj],
                        (Part::First, m) => [vi, m],
                        (Part::Second, m) => [m, vj],
                    });
                }
            } else if m != -1 {
                if part == Part::First {
                    self.touching_edges(sub, vi, m, out);
                } else {
                    self.touching_edges(sub, m, vj, out);
                }
            } else {
                self.touching_edges(sub, vi, vj, out);
            }
        }
    }

    fn contour_recursive(
        &self,
        at: i32,
        n: [i32; 3],
        propagated: [i32; 3],
        out: &mut Vec<[i32; 2]>,
    ) {
        let t = &self.triangles[at as usize];
        if !t.valid {
            return;
        }
        if t.is_split() {
            for k in 0..=usize::from(t.splits) {
                let child = self.child_neighbors(t, n, k);
                let child_p = Self::child_neighbors_propagated(t, propagated, k, child);
                self.contour_recursive(t.children[k], child, child_p, out);
            }
        } else if t.seed {
            let v = t.verts;
            self.touching_edges(n[0], v[1], v[0], out);
            self.touching_edges(n[1], v[2], v[1], out);
            self.touching_edges(n[2], v[0], v[2], out);
            for k in 0..3 {
                let other = propagated[k];
                if other != -1
                    && !self.triangles[other as usize].is_split()
                    && !self.triangles[other as usize].seed
                {
                    out.push([v[k], v[next(k)]]);
                }
            }
        }
    }

    // ---- what a project stores ----

    /// The code of every triangle of the mesh that is split or painted, by
    /// its place in the mesh, in order (`serialize`,
    /// `get_triangle_as_string`).
    pub fn serialize(&self) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        let mut digits = Vec::new();
        for at in 0..self.orig_triangles {
            let t = &self.triangles[at];
            if t.is_split() || t.state != 0 {
                digits.clear();
                self.serialize_node(at, &mut digits);
                out.push((at as u32, code::text(&digits)));
            }
        }
        out
    }

    /// The code of one triangle of the mesh; none when it is neither split
    /// nor painted.
    pub fn code_of(&self, at: usize) -> Option<String> {
        let t = &self.triangles[at];
        if !t.is_split() && t.state == 0 {
            return None;
        }
        let mut digits = Vec::new();
        self.serialize_node(at, &mut digits);
        Some(code::text(&digits))
    }

    fn serialize_node(&self, at: usize, out: &mut Vec<u8>) {
        let t = &self.triangles[at];
        if t.is_split() {
            code::push_node(
                Node::Split {
                    sides: t.splits,
                    special: t.special,
                },
                out,
            );
            for k in (0..=usize::from(t.splits)).rev() {
                self.serialize_node(t.children[k] as usize, out);
            }
        } else {
            code::push_node(Node::Leaf(t.state), out);
        }
    }

    /// Paints triangle `at` of the mesh as a code says; a code the
    /// reference drops leaves it unpainted. A state above `max_state` is
    /// read as unpainted, as the reference reads one it has no colour for
    /// (`deserialize` with `max_ebt`).
    pub fn load(&mut self, at: usize, text: &str, max_state: u8) {
        self.undivide(at as i32);
        self.triangles[at].state = 0;
        self.mark(at);
        let Some(nodes) = code::parse(text) else {
            return;
        };
        let state_of = |state: u8| if state > max_state { 0 } else { state };
        // The splits still waiting for children: the piece, its
        // neighbours, and how many of its children are done.
        struct Open {
            at: i32,
            n: [i32; 3],
            done: usize,
            total: usize,
        }
        let mut open: Vec<Open> = Vec::new();
        for node in nodes {
            let (here, n) = match open.last() {
                None => (at as i32, self.topology.neighbors[at]),
                Some(parent) => {
                    let t = self.triangles[parent.at as usize];
                    let k = parent.total - parent.done - 1;
                    (t.children[k], self.child_neighbors(&t, parent.n, k))
                }
            };
            match node {
                Node::Split { sides, special } => {
                    self.set_division(here, sides, special);
                    self.perform_split(here, n, 0);
                    open.push(Open {
                        at: here,
                        n,
                        done: 0,
                        total: usize::from(sides) + 1,
                    });
                }
                Node::Leaf(state) => {
                    self.triangles[here as usize].state = state_of(state);
                    match open.last_mut() {
                        None => return,
                        Some(parent) => parent.done += 1,
                    }
                    while open
                        .last()
                        .is_some_and(|parent| parent.done == parent.total)
                    {
                        open.pop();
                        match open.last_mut() {
                            Some(parent) => parent.done += 1,
                            None => return,
                        }
                    }
                }
            }
        }
    }

    /// Paints the mesh as a set of codes says: every other triangle is
    /// left unpainted (`deserialize`).
    pub fn load_all<'a>(&mut self, codes: impl IntoIterator<Item = (u32, &'a str)>, max_state: u8) {
        self.reset();
        for (at, text) in codes {
            if (at as usize) < self.orig_triangles {
                self.load(at as usize, text, max_state);
            }
        }
    }
}

/// TriangleSelector.cpp `is_point_inside_triangle` (barycentric, with the
/// edges counted as inside).
fn point_in_triangle(pt: V3, [p1, p2, p3]: [V3; 3]) -> bool {
    let v0 = sub(p2, p1);
    let v1 = sub(p3, p1);
    let v2 = sub(pt, p1);
    let d00 = dot(v0, v0);
    let d01 = dot(v0, v1);
    let d11 = dot(v1, v1);
    let d20 = dot(v2, v0);
    let d21 = dot(v2, v1);
    let denom = d00 * d11 - d01 * d01;
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    [u, v, w].iter().all(|c| (0.0..=1.0).contains(c))
}
