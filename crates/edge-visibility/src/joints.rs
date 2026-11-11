//! Where the parts of an object meet: points on the surface of each of
//! its parts that touch another of its parts, or a part of another object
//! stood against it. A seam hides well in such a line, and the seams of
//! an object of several parts are moved there
//! (`seam_prefer_part_joints`).
//!
//! A port of the reference's `gather_part_joints` and
//! `GlobalModelInfo::is_near_part_joint` (GCode/SeamPlacer.cpp). The
//! points are found triangle by triangle, each by itself, and kept in the
//! triangles' order, so they are the same at any thread count; and they
//! are only ever asked whether any lies near a place, which does not
//! depend on their order at all.

use rayon::prelude::*;

use crate::{Bvh, Triangles};

/// Two surfaces closer than this, in mm, and facing each other touch
/// (`PartJoints::contact_gap`).
const CONTACT_GAP: f32 = 0.2;
/// Facing each other: their normals at least about 135 degrees apart
/// (`facing_cos`).
const FACING_COS: f32 = -0.7;
/// A point of one part's surface further inside another than this, in
/// mm, touches it too (`inside_epsilon`).
const INSIDE: f32 = 0.01;
/// Only points this near another part's surface are looked at, in mm: a
/// deeper one is further than a line's width from every outer wall, and a
/// part buried well under the surface must not mark the surface above
/// it (`reach`).
const REACH: f32 = 0.5;
/// A triangle that faces this nearly straight up or down, as the cosine
/// of its slope, is passed over: a part standing on another touches it
/// all round every wall of a layer or two, which says nothing of where
/// along the wall a seam should go (`max_abs_normal_z`).
const MAX_NORMAL_Z: f32 = 0.95;
/// The spacing of the points, in mm, widened with the area to be
/// covered so that there are about so many at most (`min_spacing`,
/// `max_spacing`, `max_samples`).
const MIN_SPACING: f32 = 0.15;
const MAX_SPACING: f32 = 1.0;
const MAX_SAMPLES: f64 = 300_000.0;

/// A part, in the coordinates its object's seams are reckoned in.
#[derive(Clone, Debug)]
pub struct JointBody {
    pub mesh: Triangles,
    /// Whether it is a part of the object whose joints are wanted (only
    /// those are given points); else it is of another object.
    pub own: bool,
}

/// The points where an object's parts meet.
#[derive(Clone, Debug)]
pub struct Joints {
    points: Vec<[f32; 3]>,
    /// How far apart the points are, in mm.
    spacing: f32,
    tree: Vec<u32>,
}

/// The least and greatest corners of a box.
type Bounds = ([f64; 3], [f64; 3]);

fn bounds_of(points: impl IntoIterator<Item = [f32; 3]>) -> Bounds {
    let (mut min, mut max) = ([f64::MAX; 3], [f64::MIN; 3]);
    for point in points {
        for axis in 0..3 {
            min[axis] = min[axis].min(f64::from(point[axis]));
            max[axis] = max[axis].max(f64::from(point[axis]));
        }
    }
    (min, max)
}

fn inflated((min, max): Bounds, by: f64) -> Bounds {
    (min.map(|c| c - by), max.map(|c| c + by))
}

fn meet(a: &Bounds, b: &Bounds) -> bool {
    (0..3).all(|axis| a.0[axis] <= b.1[axis] && b.0[axis] <= a.1[axis])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn mid(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        0.5 * (a[0] + b[0]),
        0.5 * (a[1] + b[1]),
        0.5 * (a[2] + b[2]),
    ]
}

/// The unit normal of a triangle (`its_face_normal`).
fn face_normal(mesh: &Triangles, triangle: usize) -> [f32; 3] {
    let [a, b, c] = mesh.indices[triangle].map(|vertex| mesh.vertices[vertex as usize]);
    let normal = cross(sub(b, a), sub(c, a));
    let length = dot(normal, normal).sqrt();
    if length > 0.0 {
        normal.map(|c| c / length)
    } else {
        normal
    }
}

/// A part with its bounds, and the tree of its triangles once it is
/// looked in.
struct Body<'a> {
    mesh: &'a Triangles,
    bounds: Bounds,
    own: bool,
    tree: Option<Bvh>,
}

/// The nearest place on a body to a point within `reach` of it: which
/// triangle, and the place, in single precision as the reference has it
/// (`PartJoints::closest_point_within`).
fn closest_within(body: &Body<'_>, point: [f32; 3], reach: f32) -> Option<(usize, [f32; 3])> {
    let tree = body.tree.as_ref()?;
    tree.closest_within(body.mesh, point.map(f64::from), f64::from(reach))
        .map(|(triangle, on, _)| (triangle as usize, on.map(|c| c as f32)))
}

/// The points where the parts of `bodies` that are `own` touch another
/// of them; none where nothing touches. `bodies` of other objects whose
/// bounds come nowhere near those of the own parts are left out, as the
/// reference never takes them in. A port of `gather_part_joints`.
pub fn part_joints(bodies: &[JointBody]) -> Option<Joints> {
    let own_bounds = (bodies.iter())
        .filter(|body| body.own)
        .flat_map(|body| body.mesh.vertices.iter().copied())
        .collect::<Vec<_>>();
    if own_bounds.is_empty() {
        return None;
    }
    let own_reach = inflated(bounds_of(own_bounds), f64::from(REACH));
    let mut bodies: Vec<Body<'_>> = (bodies.iter())
        .filter(|body| !body.mesh.indices.is_empty())
        .map(|body| Body {
            mesh: &body.mesh,
            bounds: bounds_of(body.mesh.vertices.iter().copied()),
            own: body.own,
            tree: None,
        })
        .filter(|body| body.own || meet(&body.bounds, &own_reach))
        .collect();
    if bodies.len() < 2 {
        return None;
    }
    // Pairs of an own part and any other part whose bounds come within
    // reach of each other.
    let mut others_of: Vec<Vec<usize>> = vec![Vec::new(); bodies.len()];
    let mut needs_tree = vec![false; bodies.len()];
    for (at, body) in bodies.iter().enumerate().filter(|(_, body)| body.own) {
        let reach = inflated(body.bounds, f64::from(REACH));
        for (other, them) in bodies.iter().enumerate() {
            if other != at && meet(&reach, &them.bounds) {
                others_of[at].push(other);
                needs_tree[other] = true;
            }
        }
    }
    if others_of.iter().all(Vec::is_empty) {
        return None;
    }
    let trees: Vec<Option<Bvh>> = (bodies.par_iter().zip(&needs_tree))
        .map(|(body, needed)| needed.then(|| Bvh::new(body.mesh)))
        .collect();
    for (body, tree) in bodies.iter_mut().zip(trees) {
        body.tree = tree;
    }

    // The triangles of each own part that come within reach of another
    // part: a cheap test on a ball round each.
    let mut candidates: Vec<(usize, usize, usize)> = Vec::new();
    for (at, others) in others_of.iter().enumerate() {
        let body = &bodies[at];
        for &other in others {
            let them = &bodies[other];
            let reach = inflated(them.bounds, f64::from(REACH));
            let hit: Vec<bool> = (0..body.mesh.indices.len())
                .into_par_iter()
                .map(|triangle| {
                    let [a, b, c] = body.mesh.indices[triangle]
                        .map(|vertex| body.mesh.vertices[vertex as usize]);
                    if !meet(&bounds_of([a, b, c]), &reach) {
                        return false;
                    }
                    let normal = cross(sub(b, a), sub(c, a));
                    let length = dot(normal, normal).sqrt();
                    // No area, or facing up or down.
                    if length.is_nan() || length <= 1e-9 || normal[2].abs() > MAX_NORMAL_Z * length
                    {
                        return false;
                    }
                    let centroid = [
                        (a[0] + b[0] + c[0]) / 3.0,
                        (a[1] + b[1] + c[1]) / 3.0,
                        (a[2] + b[2] + c[2]) / 3.0,
                    ];
                    let radius = [a, b, c]
                        .map(|corner| {
                            let off = sub(corner, centroid);
                            dot(off, off)
                        })
                        .into_iter()
                        .fold(0.0f32, f32::max)
                        .sqrt();
                    closest_within(them, centroid, radius + REACH).is_some()
                })
                .collect();
            candidates.extend(
                (hit.iter().enumerate())
                    .filter(|(_, hit)| **hit)
                    .map(|(triangle, _)| (at, triangle, other)),
            );
        }
    }
    if candidates.is_empty() {
        return None;
    }

    let area: f64 = (candidates.iter())
        .map(|&(at, triangle, _)| {
            let mesh = bodies[at].mesh;
            let [a, b, c] = mesh.indices[triangle].map(|vertex| mesh.vertices[vertex as usize]);
            let normal = cross(sub(b, a), sub(c, a));
            0.5 * f64::from(dot(normal, normal).sqrt())
        })
        .sum();
    // Halving the longest edge down to `spacing` leaves pieces of about
    // a quarter of its square each.
    let spacing = ((4.0 * area / MAX_SAMPLES).sqrt() as f32).clamp(MIN_SPACING, MAX_SPACING);

    // Each candidate triangle cut by halving its longest edge until no
    // edge is longer than the spacing, a point in the middle of each
    // piece, and of those the points that touch the other part or lie
    // just inside it.
    let found: Vec<Vec<[f32; 3]>> = candidates
        .par_iter()
        .map(|&(at, triangle, other)| {
            let (body, them) = (&bodies[at], &bodies[other]);
            let normal = face_normal(body.mesh, triangle);
            let square = spacing * spacing;
            let mut kept = Vec::new();
            let mut stack =
                vec![body.mesh.indices[triangle].map(|vertex| body.mesh.vertices[vertex as usize])];
            while let Some([a, b, c]) = stack.pop() {
                let (ab, bc, ca) = (
                    dot(sub(b, a), sub(b, a)),
                    dot(sub(c, b), sub(c, b)),
                    dot(sub(a, c), sub(a, c)),
                );
                if ab.max(bc).max(ca) > square {
                    if ab >= bc && ab >= ca {
                        let m = mid(a, b);
                        stack.push([a, m, c]);
                        stack.push([m, b, c]);
                    } else if bc >= ca {
                        let m = mid(b, c);
                        stack.push([a, b, m]);
                        stack.push([a, m, c]);
                    } else {
                        let m = mid(c, a);
                        stack.push([a, b, m]);
                        stack.push([m, b, c]);
                    }
                    continue;
                }
                let sample = [
                    (a[0] + b[0] + c[0]) / 3.0,
                    (a[1] + b[1] + c[1]) / 3.0,
                    (a[2] + b[2] + c[2]) / 3.0,
                ];
                let Some((hit, on)) = closest_within(them, sample, REACH) else {
                    continue;
                };
                let their_normal = face_normal(them.mesh, hit);
                let off = sub(sample, on);
                let touching = dot(off, off) <= CONTACT_GAP * CONTACT_GAP
                    && dot(normal, their_normal) <= FACING_COS;
                let inside = dot(off, their_normal) < -INSIDE;
                if touching || inside {
                    kept.push(sample);
                }
            }
            kept
        })
        .collect();
    let points: Vec<[f32; 3]> = found.into_iter().flatten().collect();
    if points.is_empty() {
        return None;
    }
    Some(Joints::new(points, spacing))
}

impl Joints {
    /// The points, held so that those near a place are quickly found.
    pub fn new(points: Vec<[f32; 3]>, spacing: f32) -> Self {
        let mut tree: Vec<u32> = (0..points.len() as u32).collect();
        build(&points, &mut tree, 0);
        Self {
            points,
            spacing,
            tree,
        }
    }

    /// How far apart the points are, in mm.
    pub fn spacing(&self) -> f32 {
        self.spacing
    }

    /// How many points there are.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }

    /// Whether any point is within `radius` of `at`
    /// (`GlobalModelInfo::is_near_part_joint`).
    pub fn near(&self, at: [f32; 3], radius: f32) -> bool {
        near(&self.points, &self.tree, 0, at, radius * radius)
    }
}

/// Lays `order` out as a balanced tree of the points it names: the middle
/// one parts the rest across one axis, the axes taken in turn by depth.
fn build(points: &[[f32; 3]], order: &mut [u32], depth: usize) {
    if order.len() <= 1 {
        return;
    }
    let axis = depth % 3;
    let middle = order.len() / 2;
    order.select_nth_unstable_by(middle, |&a, &b| {
        points[a as usize][axis]
            .total_cmp(&points[b as usize][axis])
            .then(a.cmp(&b))
    });
    let (before, rest) = order.split_at_mut(middle);
    build(points, before, depth + 1);
    build(points, &mut rest[1..], depth + 1);
}

fn near(points: &[[f32; 3]], order: &[u32], depth: usize, at: [f32; 3], square: f32) -> bool {
    if order.is_empty() {
        return false;
    }
    let middle = order.len() / 2;
    let point = points[order[middle] as usize];
    let off = sub(at, point);
    if dot(off, off) <= square {
        return true;
    }
    let axis = depth % 3;
    let across = at[axis] - point[axis];
    let (near_side, far_side) = if across < 0.0 {
        (&order[..middle], &order[middle + 1..])
    } else {
        (&order[middle + 1..], &order[..middle])
    };
    near(points, near_side, depth + 1, at, square)
        || (across * across <= square && near(points, far_side, depth + 1, at, square))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from `min` to `max`, its triangles facing out.
    fn cuboid(min: [f32; 3], max: [f32; 3]) -> Triangles {
        let corner = |i: u32| {
            [
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            ]
        };
        Triangles {
            vertices: (0..8).map(corner).collect(),
            indices: vec![
                [0, 2, 1],
                [1, 2, 3],
                [4, 5, 6],
                [5, 7, 6],
                [0, 1, 4],
                [1, 5, 4],
                [2, 6, 3],
                [3, 6, 7],
                [0, 4, 2],
                [2, 4, 6],
                [1, 3, 5],
                [3, 7, 5],
            ],
        }
    }

    #[test]
    fn two_boxes_side_by_side_meet_where_they_touch() {
        let left = cuboid([0.0, 0.0, 0.0], [10.0, 10.0, 10.0]);
        let right = cuboid([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let joints = part_joints(&[
            JointBody {
                mesh: left.clone(),
                own: true,
            },
            JointBody {
                mesh: right.clone(),
                own: true,
            },
        ])
        .expect("the boxes touch");
        // Every point is on the face they share, which is all of it.
        assert!(joints.points.iter().all(|point| point[0] == 10.0));
        assert!(joints.near([10.4, 5.0, 5.0], 0.5));
        assert!(!joints.near([9.0, 5.0, 5.0], 0.5));
        // On the outside of the walls, the joint is the line across.
        assert!(joints.near([10.0, -0.3, 5.0], 0.4));
        assert!(!joints.near([12.0, -0.3, 5.0], 0.4));
        // The top and bottom face up and down and have no points.
        assert!(
            joints
                .points
                .iter()
                .all(|point| point[2] > 0.0 && point[2] < 10.0)
        );

        // Apart by a millimetre, they do not meet; a box alone has nothing
        // to meet; nor does a box with only a far one of another object.
        let apart = cuboid([11.0, 0.0, 0.0], [21.0, 10.0, 10.0]);
        let own = |mesh: &Triangles, own| JointBody {
            mesh: mesh.clone(),
            own,
        };
        assert!(part_joints(&[own(&left, true), own(&apart, true)]).is_none());
        assert!(part_joints(&[own(&left, true)]).is_none());
        assert!(part_joints(&[own(&left, true), own(&apart, false)]).is_none());
        // Another object's box stood against it counts; its own points
        // are not taken, only the own part's.
        let against = part_joints(&[own(&left, true), own(&right, false)]).expect("touching");
        assert!(against.points.iter().all(|point| point[0] == 10.0));
        assert!(against.len() < joints.len());
        // A box inside another: its sides are inside, its points kept,
        // but only those near the surface... of the one it is in.
        let inner = cuboid([2.0, 2.0, 2.0], [8.0, 8.0, 8.0]);
        assert!(part_joints(&[own(&left, true), own(&inner, true)]).is_none());
        let shallow = cuboid([2.0, -0.2, 2.0], [8.0, 8.0, 8.0]);
        let found = part_joints(&[own(&left, true), own(&shallow, true)]).expect("near the side");
        assert!(found.near([5.0, 0.0, 5.0], 0.5));
    }

    #[test]
    fn the_nearest_triangle_is_found_through_the_tree() {
        let mut mesh = cuboid([0.0, 0.0, 0.0], [10.0, 4.0, 6.0]);
        let more = cuboid([12.0, 1.0, 1.0], [15.0, 3.0, 9.0]);
        let offset = mesh.vertices.len() as u32;
        mesh.vertices.extend(&more.vertices);
        mesh.indices
            .extend(more.indices.iter().map(|face| face.map(|v| v + offset)));
        let tree = Bvh::new(&mesh);
        for probe in 0..400 {
            let f = probe as f64;
            let point = [
                -2.0 + 19.0 * (f * 0.618).fract(),
                -2.0 + 8.0 * (f * 0.414).fract(),
                -2.0 + 13.0 * (f * 0.732).fract(),
            ];
            let brute = (0..mesh.indices.len())
                .map(|triangle| {
                    let corners =
                        mesh.indices[triangle].map(|v| mesh.vertices[v as usize].map(f64::from));
                    let on = crate::bvh::closest_on_triangle(point, corners);
                    let off = [on[0] - point[0], on[1] - point[1], on[2] - point[2]];
                    off[0] * off[0] + off[1] * off[1] + off[2] * off[2]
                })
                .fold(f64::INFINITY, f64::min);
            for reach in [0.5, 2.0, 50.0] {
                let found = tree.closest_within(&mesh, point, reach);
                if brute <= reach * reach {
                    let (_, _, distance) = found.expect("within reach");
                    assert!(
                        (distance - brute).abs() < 1e-9,
                        "{point:?} {distance} {brute}"
                    );
                } else {
                    assert!(found.is_none(), "{point:?} {reach}");
                }
            }
        }
    }

    #[test]
    fn points_near_a_place_are_found_whatever_their_order() {
        let points: Vec<[f32; 3]> = (0..1000)
            .map(|i| {
                let f = i as f32;
                [(f * 7.31).sin() * 50.0, (f * 3.17).cos() * 50.0, f * 0.05]
            })
            .collect();
        let joints = Joints::new(points.clone(), 0.5);
        for probe in 0..300 {
            let f = probe as f32;
            let at = [(f * 1.7).sin() * 55.0, (f * 2.3).cos() * 55.0, f * 0.17];
            for radius in [0.5f32, 2.0, 6.0] {
                let brute = points.iter().any(|point| {
                    let off = sub(at, *point);
                    dot(off, off) <= radius * radius
                });
                assert_eq!(joints.near(at, radius), brute, "{at:?} {radius}");
            }
        }
    }
}
