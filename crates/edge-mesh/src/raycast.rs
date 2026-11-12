//! Which triangle of a mesh a line of sight meets first: a tree of boxes
//! round the triangles, built once for a mesh and asked many times, as the
//! reference's `MeshRaycaster` asks its AABB tree
//! (slic3r/GUI/MeshUtils.cpp `unproject_on_mesh`, libslic3r/AABBTreeIndirect.hpp).
//!
//! The tree halves the triangles at the middle of the longest side of
//! their box until a handful are left in each leaf. A ray is tested
//! against the triangles by the Möller–Trumbore test, in `f64`.

use crate::Mesh;

/// How many triangles a leaf holds at most.
const LEAF: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Node {
    low: [f32; 3],
    high: [f32; 3],
    /// For a leaf, its first triangle in `order`; for a split, its second
    /// child (the first follows it).
    at: u32,
    /// For a leaf, how many triangles; 0 for a split.
    count: u32,
}

/// A mesh's tree of boxes.
#[derive(Debug)]
pub struct Tree {
    nodes: Vec<Node>,
    order: Vec<u32>,
    corners: Vec<[[f64; 3]; 3]>,
}

/// Where a ray meets a mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    pub triangle: usize,
    /// How far along the ray, in its direction's lengths.
    pub t: f64,
    pub point: [f64; 3],
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl Tree {
    pub fn new(mesh: &Mesh) -> Tree {
        let corners: Vec<[[f64; 3]; 3]> = (0..mesh.triangles.len())
            .map(|at| mesh.triangle(at).map(|p| p.map(f64::from)))
            .collect();
        let centres: Vec<[f32; 3]> = (0..mesh.triangles.len())
            .map(|at| {
                let [a, b, c] = mesh.triangle(at);
                std::array::from_fn(|axis| (a[axis] + b[axis] + c[axis]) / 3.0)
            })
            .collect();
        let mut order: Vec<u32> = (0..mesh.triangles.len() as u32).collect();
        let mut nodes = Vec::with_capacity(2 * mesh.triangles.len() / LEAF + 1);
        if !order.is_empty() {
            build(mesh, &centres, &mut order, 0, &mut nodes);
        }
        Tree {
            nodes,
            order,
            corners,
        }
    }

    /// The first triangle `origin + t · direction` meets for `t` in
    /// `range`, and where; triangles met from behind count. `keep` may
    /// turn a hit down (one cut away by a section view), and the next is
    /// looked for.
    pub fn hit(
        &self,
        origin: [f64; 3],
        direction: [f64; 3],
        range: (f64, f64),
        keep: &dyn Fn([f64; 3]) -> bool,
    ) -> Option<Hit> {
        if self.nodes.is_empty() {
            return None;
        }
        let inverse = direction.map(|d| 1.0 / d);
        let mut best: Option<Hit> = None;
        let mut stack: Vec<u32> = vec![0];
        while let Some(at) = stack.pop() {
            let node = self.nodes[at as usize];
            let far = best.map_or(range.1, |hit| hit.t);
            if !box_hit(&node, origin, inverse, range.0, far) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.at);
                stack.push(at + 1);
                continue;
            }
            for &triangle in &self.order[node.at as usize..(node.at + node.count) as usize] {
                let Some(t) = triangle_hit(&self.corners[triangle as usize], origin, direction)
                else {
                    continue;
                };
                let far = best.map_or(range.1, |hit| hit.t);
                if t < range.0 || t > far {
                    continue;
                }
                let point = std::array::from_fn(|axis| origin[axis] + t * direction[axis]);
                if !keep(point) {
                    continue;
                }
                // Equal distances go to the lower triangle, whatever order
                // they are met in.
                if best.is_none_or(|hit| {
                    t < hit.t || (t == hit.t && (triangle as usize) < hit.triangle)
                }) {
                    best = Some(Hit {
                        triangle: triangle as usize,
                        t,
                        point,
                    });
                }
            }
        }
        best
    }

    /// The triangle nearest a point (`get_closest_facet`), by the distance to
    /// the triangle.
    pub fn closest(&self, point: [f64; 3]) -> Option<usize> {
        let mut best: Option<(f64, usize)> = None;
        let mut stack: Vec<u32> = if self.nodes.is_empty() {
            Vec::new()
        } else {
            vec![0]
        };
        while let Some(at) = stack.pop() {
            let node = self.nodes[at as usize];
            let d = box_distance_sqr(&node, point);
            if best.is_some_and(|(least, _)| d > least) {
                continue;
            }
            if node.count == 0 {
                stack.push(node.at);
                stack.push(at + 1);
                continue;
            }
            for &triangle in &self.order[node.at as usize..(node.at + node.count) as usize] {
                let d = point_triangle_distance_sqr(point, &self.corners[triangle as usize]);
                if best
                    .is_none_or(|(least, at)| d < least || (d == least && (triangle as usize) < at))
                {
                    best = Some((d, triangle as usize));
                }
            }
        }
        best.map(|(_, at)| at)
    }
}

fn build(
    mesh: &Mesh,
    centres: &[[f32; 3]],
    order: &mut [u32],
    first: u32,
    nodes: &mut Vec<Node>,
) -> u32 {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];
    for &t in order.iter() {
        for p in mesh.triangle(t as usize) {
            for axis in 0..3 {
                low[axis] = low[axis].min(p[axis]);
                high[axis] = high[axis].max(p[axis]);
            }
        }
    }
    let at = nodes.len() as u32;
    nodes.push(Node {
        low,
        high,
        at: first,
        count: order.len() as u32,
    });
    if order.len() <= LEAF {
        return at;
    }
    // Halved at the middle by count along the longest side of the box of
    // the middles.
    let mut c_low = [f32::INFINITY; 3];
    let mut c_high = [f32::NEG_INFINITY; 3];
    for &t in order.iter() {
        for axis in 0..3 {
            c_low[axis] = c_low[axis].min(centres[t as usize][axis]);
            c_high[axis] = c_high[axis].max(centres[t as usize][axis]);
        }
    }
    let axis = (0..3)
        .max_by(|&a, &b| (c_high[a] - c_low[a]).total_cmp(&(c_high[b] - c_low[b])))
        .unwrap_or(0);
    let half = order.len() / 2;
    order.select_nth_unstable_by(half, |&a, &b| {
        centres[a as usize][axis]
            .total_cmp(&centres[b as usize][axis])
            .then(a.cmp(&b))
    });
    let (left, right) = order.split_at_mut(half);
    build(mesh, centres, left, first, nodes);
    let second = build(mesh, centres, right, first + half as u32, nodes);
    nodes[at as usize].at = second;
    nodes[at as usize].count = 0;
    at
}

fn box_hit(node: &Node, origin: [f64; 3], inverse: [f64; 3], near: f64, far: f64) -> bool {
    let (mut t0, mut t1) = (near, far);
    for axis in 0..3 {
        let a = (f64::from(node.low[axis]) - origin[axis]) * inverse[axis];
        let b = (f64::from(node.high[axis]) - origin[axis]) * inverse[axis];
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        // A ray along the box's side gives NaN: it is taken as inside.
        if a.is_nan() || b.is_nan() {
            if origin[axis] < f64::from(node.low[axis]) || origin[axis] > f64::from(node.high[axis])
            {
                return false;
            }
            continue;
        }
        // A little slack for a ray along a side of the box; an infinite
        // bound (a ray square to this axis) stays as it is.
        let slack = |value: f64| {
            if value.is_finite() {
                1e-9 * value.abs()
            } else {
                0.0
            }
        };
        t0 = t0.max(a - slack(a));
        t1 = t1.min(b + slack(b));
        if t0 > t1 {
            return false;
        }
    }
    true
}

/// Möller–Trumbore.
fn triangle_hit(corners: &[[f64; 3]; 3], origin: [f64; 3], direction: [f64; 3]) -> Option<f64> {
    let e1 = sub(corners[1], corners[0]);
    let e2 = sub(corners[2], corners[0]);
    let p = cross(direction, e2);
    let det = dot(e1, p);
    if det.abs() < 1e-18 {
        return None;
    }
    let inv = 1.0 / det;
    let s = sub(origin, corners[0]);
    let u = dot(s, p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross(s, e1);
    let v = dot(direction, q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    Some(dot(e2, q) * inv)
}

fn box_distance_sqr(node: &Node, p: [f64; 3]) -> f64 {
    (0..3)
        .map(|axis| {
            let d = (f64::from(node.low[axis]) - p[axis])
                .max(p[axis] - f64::from(node.high[axis]))
                .max(0.0);
            d * d
        })
        .sum()
}

/// The square of the distance from a point to a triangle (Ericson,
/// Real-Time Collision Detection, 5.1.5).
fn point_triangle_distance_sqr(p: [f64; 3], [a, b, c]: &[[f64; 3]; 3]) -> f64 {
    let closest = {
        let ab = sub(*b, *a);
        let ac = sub(*c, *a);
        let ap = sub(p, *a);
        let d1 = dot(ab, ap);
        let d2 = dot(ac, ap);
        let at = |base: [f64; 3], dir: [f64; 3], t: f64| -> [f64; 3] {
            std::array::from_fn(|i| base[i] + t * dir[i])
        };
        if d1 <= 0.0 && d2 <= 0.0 {
            *a
        } else {
            let bp = sub(p, *b);
            let d3 = dot(ab, bp);
            let d4 = dot(ac, bp);
            if d3 >= 0.0 && d4 <= d3 {
                *b
            } else {
                let vc = d1 * d4 - d3 * d2;
                if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
                    at(*a, ab, d1 / (d1 - d3))
                } else {
                    let cp = sub(p, *c);
                    let d5 = dot(ab, cp);
                    let d6 = dot(ac, cp);
                    if d6 >= 0.0 && d5 <= d6 {
                        *c
                    } else {
                        let vb = d5 * d2 - d1 * d6;
                        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
                            at(*a, ac, d2 / (d2 - d6))
                        } else {
                            let va = d3 * d6 - d5 * d4;
                            if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
                                at(*b, sub(*c, *b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
                            } else {
                                let denom = 1.0 / (va + vb + vc);
                                let v = vb * denom;
                                let w = vc * denom;
                                std::array::from_fn(|i| a[i] + ab[i] * v + ac[i] * w)
                            }
                        }
                    }
                }
            }
        }
    };
    let d = sub(p, closest);
    dot(d, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere() -> Mesh {
        // A UV sphere of radius 10, many triangles.
        let (rings, segments) = (60usize, 120usize);
        let point = |r: usize, s: usize| -> [f32; 3] {
            let theta = std::f32::consts::PI * r as f32 / rings as f32;
            let phi = 2.0 * std::f32::consts::PI * s as f32 / segments as f32;
            [
                10.0 * theta.sin() * phi.cos(),
                10.0 * theta.sin() * phi.sin(),
                10.0 * theta.cos(),
            ]
        };
        let mut soup = Vec::new();
        for r in 0..rings {
            for s in 0..segments {
                let (a, b, c, d) = (
                    point(r, s),
                    point(r + 1, s),
                    point(r + 1, s + 1),
                    point(r, s + 1),
                );
                soup.push([a, b, c]);
                soup.push([a, c, d]);
            }
        }
        Mesh::from_triangle_soup(soup)
    }

    #[test]
    fn the_first_hit_is_the_nearest_and_is_the_brute_forces() {
        let mesh = sphere();
        let tree = Tree::new(&mesh);
        for (origin, direction) in [
            ([0.0, 0.0, 50.0], [0.0, 0.0, -1.0]),
            ([30.0, 3.0, 2.0], [-1.0, 0.0, 0.0]),
            ([25.0, 25.0, 25.0], [-1.0, -1.0, -1.0]),
            ([5.0, 50.0, -3.0], [0.0, -1.0, 0.1]),
        ] {
            let hit = tree
                .hit(origin, direction, (0.0, f64::INFINITY), &|_| true)
                .unwrap();
            // Every triangle tried.
            let brute = (0..mesh.triangles.len())
                .filter_map(|at| {
                    triangle_hit(&tree.corners[at], origin, direction).map(|t| (t, at))
                })
                .filter(|(t, _)| *t >= 0.0)
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .unwrap();
            assert_eq!(hit.t, brute.0);
            let r = hit.point.iter().map(|c| c * c).sum::<f64>().sqrt();
            assert!((r - 10.0).abs() < 0.1, "{r}");
            let near = tree.closest(hit.point).unwrap();
            assert!(point_triangle_distance_sqr(hit.point, &tree.corners[near]) < 1e-12);
        }
        // A miss.
        assert!(
            tree.hit(
                [0.0, 0.0, 50.0],
                [0.0, 1.0, 0.0],
                (0.0, f64::INFINITY),
                &|_| true
            )
            .is_none()
        );
        // What is turned down is passed through: the far side is met.
        let far = tree
            .hit(
                [0.0, 0.0, 50.0],
                [0.0, 0.0, -1.0],
                (0.0, f64::INFINITY),
                &|p| p[2] < 0.0,
            )
            .unwrap();
        assert!(far.point[2] < -9.9);
    }
}
