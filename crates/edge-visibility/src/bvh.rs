//! Rays cast at a mesh: its triangles in a tree of boxes, so that a ray
//! is tried against few of them.
//!
//! The reference casts its rays through `AABBTreeIndirect`
//! (AABBTreeIndirect.hpp), whose triangle test is ported here as it is
//! (`intersect_triangle`, after Möller's `intersect_triangle1`, in double
//! precision with its epsilon of a millionth). The tree is this crate's
//! own: which triangle a ray meets first does not depend on the tree but
//! where two are met at the very same distance.

use crate::Triangles;

/// A place a ray meets the mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Which triangle.
    pub triangle: u32,
    /// How far along the ray, in lengths of its direction; kept in single
    /// precision as the reference keeps it (`igl::Hit`).
    pub t: f32,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    min: [f64; 3],
    max: [f64; 3],
    /// For a leaf, where its triangles start in `order` and how many;
    /// else its second child (the first follows it) and 0.
    at: u32,
    count: u32,
}

/// A mesh's triangles in a tree of boxes.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
}

/// Triangles in a leaf, at most.
const LEAF: usize = 4;
/// The reference's epsilon for a ray that runs along a triangle's plane.
const EPSILON: f64 = 0.000_001;

fn corner(mesh: &Triangles, triangle: u32, which: usize) -> [f64; 3] {
    mesh.vertices[mesh.indices[triangle as usize][which] as usize].map(f64::from)
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// How far along a ray it meets a triangle, if it does, before or behind
/// its origin (AABBTreeIndirect.hpp `intersect_triangle`).
fn intersect(origin: [f64; 3], direction: [f64; 3], corners: [[f64; 3]; 3]) -> Option<f64> {
    let edge1 = sub(corners[1], corners[0]);
    let edge2 = sub(corners[2], corners[0]);
    let pvec = cross(direction, edge2);
    let det = dot(edge1, pvec);
    let tvec = sub(origin, corners[0]);
    let qvec;
    if det > EPSILON {
        let u = dot(tvec, pvec);
        if u < 0.0 || u > det {
            return None;
        }
        qvec = cross(tvec, edge1);
        let v = dot(direction, qvec);
        if v < 0.0 || u + v > det {
            return None;
        }
    } else if det < -EPSILON {
        let u = dot(tvec, pvec);
        if u > 0.0 || u < det {
            return None;
        }
        qvec = cross(tvec, edge1);
        let v = dot(direction, qvec);
        if v > 0.0 || u + v < det {
            return None;
        }
    } else {
        // The ray runs along the triangle's plane.
        return None;
    }
    Some(dot(edge2, qvec) * (1.0 / det))
}

impl Bvh {
    pub fn new(mesh: &Triangles) -> Self {
        let count = mesh.indices.len();
        let mut bvh = Self {
            nodes: Vec::with_capacity(count / 2 + 1),
            order: (0..count as u32).collect(),
        };
        if count == 0 {
            return bvh;
        }
        let boxes: Vec<([f64; 3], [f64; 3])> = (0..count as u32)
            .map(|triangle| {
                let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
                for which in 0..3 {
                    let at = corner(mesh, triangle, which);
                    for axis in 0..3 {
                        min[axis] = min[axis].min(at[axis]);
                        max[axis] = max[axis].max(at[axis]);
                    }
                }
                (min, max)
            })
            .collect();
        bvh.build(&boxes, 0, count);
        bvh
    }

    /// Makes the node of `order[from..to]` and all beneath it; gives its
    /// place.
    fn build(&mut self, boxes: &[([f64; 3], [f64; 3])], from: usize, to: usize) -> usize {
        let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
        let (mut low, mut high) = ([f64::MAX; 3], [f64::MIN; 3]);
        for &triangle in &self.order[from..to] {
            let (least, most) = boxes[triangle as usize];
            for axis in 0..3 {
                min[axis] = min[axis].min(least[axis]);
                max[axis] = max[axis].max(most[axis]);
                let middle = least[axis] + most[axis];
                low[axis] = low[axis].min(middle);
                high[axis] = high[axis].max(middle);
            }
        }
        let here = self.nodes.len();
        self.nodes.push(Node {
            min,
            max,
            at: from as u32,
            count: (to - from) as u32,
        });
        if to - from <= LEAF {
            return here;
        }
        // Halved across the axis the triangles' middles spread furthest
        // along.
        let axis = (0..3)
            .max_by(|&a, &b| {
                (high[a] - low[a])
                    .total_cmp(&(high[b] - low[b]))
                    .then(b.cmp(&a))
            })
            .unwrap_or(0);
        let half = from + (to - from) / 2;
        let key = |triangle: &u32| {
            let (least, most) = boxes[*triangle as usize];
            least[axis] + most[axis]
        };
        self.order[from..to]
            .select_nth_unstable_by(half - from, |a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
        self.build(boxes, from, half);
        let second = self.build(boxes, half, to);
        self.nodes[here].at = second as u32;
        self.nodes[here].count = 0;
        here
    }

    /// Whether a ray reaches a box before `far`
    /// (`ray_box_intersect_invdir`).
    fn reaches(node: &Node, origin: [f64; 3], inverse: [f64; 3], far: f64) -> bool {
        let (mut nearest, mut furthest) = (f64::NEG_INFINITY, f64::INFINITY);
        for axis in 0..3 {
            let (mut enter, mut leave) = (
                (node.min[axis] - origin[axis]) * inverse[axis],
                (node.max[axis] - origin[axis]) * inverse[axis],
            );
            if inverse[axis] < 0.0 {
                std::mem::swap(&mut enter, &mut leave);
            }
            // A ray along a face of the box gives 0 times infinity,
            // which is no number: such a face rules nothing out.
            if enter > nearest {
                nearest = enter;
            }
            if leave < furthest {
                furthest = leave;
            }
            if nearest > furthest {
                return false;
            }
        }
        nearest < far && furthest > 0.0
    }

    /// Every triangle a ray meets ahead of its origin, as they are come
    /// to in the tree.
    fn each_hit(
        &self,
        mesh: &Triangles,
        origin: [f64; 3],
        direction: [f64; 3],
        mut found: impl FnMut(Hit) -> f64,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let inverse = direction.map(|d| 1.0 / d);
        let mut far = f64::INFINITY;
        // The tree halves its triangles at every node, so it is no
        // deeper than the bits of a count of them.
        let mut stack = [0usize; 64];
        let mut held = 1;
        while held > 0 {
            held -= 1;
            let at = stack[held];
            let node = &self.nodes[at];
            if !Self::reaches(node, origin, inverse, far) {
                continue;
            }
            if node.count == 0 {
                stack[held] = node.at as usize;
                stack[held + 1] = at + 1;
                held += 2;
                continue;
            }
            let from = node.at as usize;
            for &triangle in &self.order[from..from + node.count as usize] {
                let corners = [0, 1, 2].map(|which| corner(mesh, triangle, which));
                if let Some(t) = intersect(origin, direction, corners).filter(|t| *t > 0.0) {
                    far = found(Hit {
                        triangle,
                        t: t as f32,
                    });
                }
            }
        }
    }

    /// The first triangle a ray meets ahead of its origin
    /// (`intersect_ray_first_hit`); of two at the very same distance,
    /// the one of the lower number.
    pub fn first_hit(
        &self,
        mesh: &Triangles,
        origin: [f64; 3],
        direction: [f64; 3],
    ) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        self.each_hit(mesh, origin, direction, |hit| {
            if best.is_none_or(|kept| {
                hit.t < kept.t || (hit.t == kept.t && hit.triangle < kept.triangle)
            }) {
                best = Some(hit);
            }
            // Nothing further than the nearest so far need be tried; a
            // box as far is, for the tie.
            best.map_or(f64::INFINITY, |kept| f64::from(kept.t).next_up())
        });
        best
    }

    /// Every triangle a ray meets ahead of its origin, nearest first
    /// (`intersect_ray_all_hits`).
    pub fn all_hits(&self, mesh: &Triangles, origin: [f64; 3], direction: [f64; 3]) -> Vec<Hit> {
        let mut hits = Vec::new();
        self.each_hit(mesh, origin, direction, |hit| {
            hits.push(hit);
            f64::INFINITY
        });
        hits.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.triangle.cmp(&b.triangle)));
        hits
    }
}
