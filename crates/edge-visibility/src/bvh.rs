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

#[derive(Clone, Copy, Debug, PartialEq)]
struct Node {
    min: [f64; 3],
    max: [f64; 3],
    /// For a leaf, where its triangles start in `order` and how many;
    /// else its second child (the first follows it) and 0.
    at: u32,
    count: u32,
}

/// A mesh's triangles in a tree of boxes. The tree holds the corners of
/// the triangles it was made of; the mesh its queries take is that one,
/// for the numbers of the triangles it gives back.
#[derive(Clone, Debug, Default)]
pub struct Bvh {
    nodes: Vec<Node>,
    order: Vec<u32>,
    /// The corners of the triangle at each place in `order`, in double
    /// precision: what a ray is tried against, read in the order the
    /// leaves hold them and not looked up through the mesh each time.
    corners: Vec<[[f64; 3]; 3]>,
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

/// The place on a triangle nearest a point (after Ericson, Real-Time
/// Collision Detection, 5.1.5).
pub(crate) fn closest_on_triangle(p: [f64; 3], [a, b, c]: [[f64; 3]; 3]) -> [f64; 3] {
    let along = |from: [f64; 3], by: [f64; 3], t: f64| {
        [
            from[0] + by[0] * t,
            from[1] + by[1] * t,
            from[2] + by[2] * t,
        ]
    };
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return along(a, ab, d1 / (d1 - d3));
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return along(a, ac, d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denominator = va + vb + vc;
    if denominator == 0.0 {
        // No area: the nearest of its corners.
        return [a, b, c]
            .into_iter()
            .min_by(|x, y| {
                let (dx, dy) = (sub(*x, p), sub(*y, p));
                dot(dx, dx).total_cmp(&dot(dy, dy))
            })
            .unwrap_or(a);
    }
    let (v, w) = (vb / denominator, vc / denominator);
    [
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ]
}

/// Above this many triangles the two halves of a node are made side by
/// side; the tree is the same either way.
const SIDE_BY_SIDE: usize = 1 << 13;

/// Makes the node of `order` (the triangles from `from` on in the tree's
/// order) and all beneath it at the end of `nodes`, the first child
/// straight after its parent and then the second's; gives its place.
fn grow(
    nodes: &mut Vec<Node>,
    order: &mut [u32],
    from: usize,
    boxes: &[([f64; 3], [f64; 3])],
    side_by_side: usize,
) -> usize {
    let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
    let (mut low, mut high) = ([f64::MAX; 3], [f64::MIN; 3]);
    for &triangle in order.iter() {
        let (least, most) = boxes[triangle as usize];
        for axis in 0..3 {
            min[axis] = min[axis].min(least[axis]);
            max[axis] = max[axis].max(most[axis]);
            let middle = least[axis] + most[axis];
            low[axis] = low[axis].min(middle);
            high[axis] = high[axis].max(middle);
        }
    }
    let here = nodes.len();
    nodes.push(Node {
        min,
        max,
        at: from as u32,
        count: order.len() as u32,
    });
    if order.len() <= LEAF {
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
    let half = order.len() / 2;
    let key = |triangle: &u32| {
        let (least, most) = boxes[*triangle as usize];
        least[axis] + most[axis]
    };
    order.select_nth_unstable_by(half, |a, b| key(a).total_cmp(&key(b)).then(a.cmp(b)));
    let large = order.len() > side_by_side;
    let (first, second) = order.split_at_mut(half);
    let second = if large {
        // Each half in a list of its own, its places counted from its
        // own start, then put after the parent with them moved on.
        let (mut made_first, mut made_second) = rayon::join(
            || {
                let mut made = Vec::new();
                grow(&mut made, first, from, boxes, side_by_side);
                made
            },
            || {
                let mut made = Vec::new();
                grow(&mut made, second, from + half, boxes, side_by_side);
                made
            },
        );
        let moved = |made: &mut Vec<Node>, by: usize| {
            for node in made.iter_mut().filter(|node| node.count == 0) {
                node.at += by as u32;
            }
        };
        moved(&mut made_first, nodes.len());
        nodes.append(&mut made_first);
        let second = nodes.len();
        moved(&mut made_second, second);
        nodes.append(&mut made_second);
        second
    } else {
        grow(nodes, first, from, boxes, side_by_side);
        grow(nodes, second, from + half, boxes, side_by_side)
    };
    nodes[here].at = second as u32;
    nodes[here].count = 0;
    here
}

impl Bvh {
    pub fn new(mesh: &Triangles) -> Self {
        Self::made(mesh, SIDE_BY_SIDE)
    }

    fn made(mesh: &Triangles, side_by_side: usize) -> Self {
        let count = mesh.indices.len();
        let mut bvh = Self {
            nodes: Vec::with_capacity(count / 2 + 1),
            order: (0..count as u32).collect(),
            corners: Vec::new(),
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
        let mut order = std::mem::take(&mut bvh.order);
        grow(&mut bvh.nodes, &mut order, 0, &boxes, side_by_side);
        bvh.corners = order
            .iter()
            .map(|&triangle| [0, 1, 2].map(|which| corner(mesh, triangle, which)))
            .collect();
        bvh.order = order;
        bvh
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
        origin: [f64; 3],
        direction: [f64; 3],
        reach: f64,
        mut found: impl FnMut(Hit) -> f64,
    ) {
        if self.nodes.is_empty() {
            return;
        }
        let inverse = direction.map(|d| 1.0 / d);
        let mut far = reach;
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
            let to = from + node.count as usize;
            for (&triangle, &corners) in self.order[from..to].iter().zip(&self.corners[from..to]) {
                if let Some(t) =
                    intersect(origin, direction, corners).filter(|t| *t > 0.0 && *t <= reach)
                {
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
        self.first_hit_within(mesh, origin, direction, f64::INFINITY)
    }

    /// [`Self::first_hit`], if it is no further along the ray than
    /// `reach`: no box beyond that is looked in.
    pub fn first_hit_within(
        &self,
        _mesh: &Triangles,
        origin: [f64; 3],
        direction: [f64; 3],
        reach: f64,
    ) -> Option<Hit> {
        let mut best: Option<Hit> = None;
        self.each_hit(origin, direction, reach, |hit| {
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

    /// The triangle nearest a point, if one is within `reach` of it,
    /// with the nearest place on it and the square of the distance; of
    /// two as near, the one of the lower number (AABBTreeIndirect.hpp
    /// `squared_distance_to_indexed_primitives_recursive`).
    pub fn closest_within(
        &self,
        _mesh: &Triangles,
        point: [f64; 3],
        reach: f64,
    ) -> Option<(u32, [f64; 3], f64)> {
        if self.nodes.is_empty() {
            return None;
        }
        let box_distance = |node: &Node| {
            (0..3)
                .map(|axis| {
                    let away = (node.min[axis] - point[axis])
                        .max(point[axis] - node.max[axis])
                        .max(0.0);
                    away * away
                })
                .sum::<f64>()
        };
        let mut nearest = reach * reach;
        let mut best: Option<(u32, [f64; 3], f64)> = None;
        let mut stack = [0usize; 64];
        let mut held = 1;
        while held > 0 {
            held -= 1;
            let at = stack[held];
            let node = &self.nodes[at];
            if box_distance(node) > nearest {
                continue;
            }
            if node.count == 0 {
                // The nearer child is looked in first.
                let (first, second) = (at + 1, node.at as usize);
                let (near, far) =
                    if box_distance(&self.nodes[first]) <= box_distance(&self.nodes[second]) {
                        (first, second)
                    } else {
                        (second, first)
                    };
                stack[held] = far;
                stack[held + 1] = near;
                held += 2;
                continue;
            }
            let from = node.at as usize;
            let to = from + node.count as usize;
            for (&triangle, &corners) in self.order[from..to].iter().zip(&self.corners[from..to]) {
                let on = closest_on_triangle(point, corners);
                let away = sub(on, point);
                let distance = dot(away, away);
                let better = match best {
                    None => distance <= nearest,
                    Some((kept, _, kept_distance)) => {
                        distance < kept_distance || (distance == kept_distance && triangle < kept)
                    }
                };
                if better {
                    best = Some((triangle, on, distance));
                    nearest = distance;
                }
            }
        }
        best
    }

    /// Whether any triangle is within `reach` of a point: what
    /// [`Self::closest_within`] finds something for, without looking on
    /// for the nearest once one is found.
    pub fn any_within(&self, point: [f64; 3], reach: f64) -> bool {
        let square = reach * reach;
        let box_distance = |node: &Node| {
            (0..3)
                .map(|axis| {
                    let away = (node.min[axis] - point[axis])
                        .max(point[axis] - node.max[axis])
                        .max(0.0);
                    away * away
                })
                .sum::<f64>()
        };
        let mut stack = [0usize; 64];
        let mut held = usize::from(!self.nodes.is_empty());
        while held > 0 {
            held -= 1;
            let at = stack[held];
            let node = &self.nodes[at];
            if box_distance(node) > square {
                continue;
            }
            if node.count == 0 {
                stack[held] = node.at as usize;
                stack[held + 1] = at + 1;
                held += 2;
                continue;
            }
            let from = node.at as usize;
            for &corners in &self.corners[from..from + node.count as usize] {
                let away = sub(closest_on_triangle(point, corners), point);
                if dot(away, away) <= square {
                    return true;
                }
            }
        }
        false
    }

    /// Every triangle a ray meets ahead of its origin, nearest first
    /// (`intersect_ray_all_hits`).
    pub fn all_hits(&self, _mesh: &Triangles, origin: [f64; 3], direction: [f64; 3]) -> Vec<Hit> {
        let mut hits = Vec::new();
        self.each_hit(origin, direction, f64::INFINITY, |hit| {
            hits.push(hit);
            f64::INFINITY
        });
        hits.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.triangle.cmp(&b.triangle)));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of little tilted squares, `n` by `n`, two triangles each.
    fn tiles(n: usize) -> Triangles {
        let mut mesh = Triangles::default();
        for i in 0..n {
            for j in 0..n {
                let (x, y) = (i as f32, j as f32);
                let z = ((i * 7 + j * 3) % 11) as f32 * 0.1;
                let at = mesh.vertices.len() as u32;
                mesh.vertices.extend([
                    [x, y, z],
                    [x + 1.0, y, z + 0.2],
                    [x + 1.0, y + 1.0, z],
                    [x, y + 1.0, z + 0.1],
                ]);
                mesh.indices
                    .extend([[at, at + 1, at + 2], [at, at + 2, at + 3]]);
            }
        }
        mesh
    }

    #[test]
    fn halves_made_side_by_side_make_the_same_tree() {
        let mesh = tiles(120);
        assert!(mesh.indices.len() > 4 * SIDE_BY_SIDE / 2);
        let alone = Bvh::made(&mesh, usize::MAX);
        let together = Bvh::new(&mesh);
        assert_eq!(alone.nodes, together.nodes);
        assert_eq!(alone.order, together.order);
        // And a ray finds what it found, and nothing past its reach.
        let hit = together.first_hit(&mesh, [10.5, 20.5, -1.0], [0.0, 0.0, 1.0]);
        assert_eq!(
            hit,
            alone.first_hit(&mesh, [10.5, 20.5, -1.0], [0.0, 0.0, 1.0])
        );
        let t = f64::from(hit.unwrap().t);
        assert_eq!(
            together.first_hit_within(&mesh, [10.5, 20.5, -1.0], [0.0, 0.0, 1.0], t + 1e-3),
            hit
        );
        assert_eq!(
            together.first_hit_within(&mesh, [10.5, 20.5, -1.0], [0.0, 0.0, 1.0], t - 1e-3),
            None
        );
    }
}
