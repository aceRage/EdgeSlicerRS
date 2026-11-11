//! The convex hull of points in three dimensions: the smallest convex solid
//! that holds them all, as a closed mesh of outward-facing triangles.
//!
//! The reference takes it from qhull (TriangleMesh.cpp `its_convex_hull`).
//! This is an incremental hull of its own (quickhull: each step takes the
//! point farthest outside a face, removes the faces it sees and closes the
//! hole with faces to it), in `f64`, with the same answer on every run: no
//! hashing order, no randomness, ties broken by the lower index. Points
//! within a small distance of a face (a millionth of the cloud's size) are
//! taken as lying on it, so a face that is flat in the model but rounded to
//! `f32` comes out as one plane cut into triangles, not a fan of slivers.
//! Which triangles a flat face is cut into is not the reference's, and
//! nothing here depends on it: whoever reads the hull groups its
//! triangles by plane.

use rustc_hash::FxHashMap;

/// A convex hull: its corners and its triangles, each wound
/// counter-clockwise seen from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct Hull {
    pub points: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
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

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

/// One face while the hull is built.
struct Face {
    corners: [usize; 3],
    /// Outward unit normal and the plane's distance from the origin.
    normal: [f64; 3],
    offset: f64,
    /// The points outside it that no earlier face has taken.
    outside: Vec<usize>,
    alive: bool,
}

impl Face {
    fn new(points: &[[f64; 3]], corners: [usize; 3]) -> Self {
        let [a, b, c] = corners.map(|at| points[at]);
        let n = cross(sub(b, a), sub(c, a));
        let size = length(n);
        let normal = if size > 0.0 {
            n.map(|value| value / size)
        } else {
            [0.0; 3]
        };
        Self {
            corners,
            normal,
            offset: dot(normal, a),
            outside: Vec::new(),
            alive: true,
        }
    }

    fn distance(&self, point: [f64; 3]) -> f64 {
        dot(self.normal, point) - self.offset
    }
}

/// The hull of `points`; none when they do not span a solid (fewer than
/// four, or all on one plane).
pub fn convex_hull(points: &[[f64; 3]]) -> Option<Hull> {
    if points.len() < 4 {
        return None;
    }
    // How near a plane a point may be and still lie on it.
    let (low, high) = points.iter().fold(
        ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
        |(mut low, mut high), point| {
            for axis in 0..3 {
                low[axis] = low[axis].min(point[axis]);
                high[axis] = high[axis].max(point[axis]);
            }
            (low, high)
        },
    );
    let reach = (0..3)
        .map(|axis| low[axis].abs().max(high[axis].abs()))
        .fold(0.0, f64::max);
    let size = length(sub(high, low));
    if size.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) || !reach.is_finite() {
        return None;
    }
    let eps = 1e-6 * size + 1e-12 * reach;

    // The first solid: the two points farthest apart along an axis, the
    // point farthest from the line through them, the point farthest from
    // the plane through those three.
    let axis = (0..3)
        .max_by(|a, b| {
            (high[*a] - low[*a])
                .total_cmp(&(high[*b] - low[*b]))
                .then(b.cmp(a))
        })
        .expect("three axes");
    let first_by = |better: &dyn Fn(usize, usize) -> bool| {
        (0..points.len()).fold(0, |best, at| if better(at, best) { at } else { best })
    };
    let a = first_by(&|at, best| points[at][axis] < points[best][axis]);
    let b = first_by(&|at, best| points[at][axis] > points[best][axis]);
    if a == b {
        return None;
    }
    let line = sub(points[b], points[a]);
    let off_line = |at: usize| length(cross(line, sub(points[at], points[a])));
    let c = first_by(&|at, best| off_line(at) > off_line(best));
    if off_line(c) <= eps * length(line) {
        return None;
    }
    let plane = Face::new(points, [a, b, c]);
    let d =
        first_by(&|at, best| plane.distance(points[at]).abs() > plane.distance(points[best]).abs());
    if plane.distance(points[d]).abs() <= eps {
        return None;
    }

    let mut faces: Vec<Face> = Vec::new();
    // Which face each directed edge belongs to.
    let mut edges: FxHashMap<(usize, usize), usize> = FxHashMap::default();
    let add = |faces: &mut Vec<Face>,
               edges: &mut FxHashMap<(usize, usize), usize>,
               corners: [usize; 3]|
     -> usize {
        let at = faces.len();
        for k in 0..3 {
            edges.insert((corners[k], corners[(k + 1) % 3]), at);
        }
        faces.push(Face::new(points, corners));
        at
    };
    // The four faces of the first solid, each turned to face away from
    // the fourth corner.
    for (corners, other) in [
        ([a, b, c], d),
        ([a, b, d], c),
        ([a, c, d], b),
        ([b, c, d], a),
    ] {
        let face = Face::new(points, corners);
        let corners = if face.distance(points[other]) > 0.0 {
            [corners[0], corners[2], corners[1]]
        } else {
            corners
        };
        add(&mut faces, &mut edges, corners);
    }
    let first = [a, b, c, d];
    for (at, point) in points.iter().enumerate() {
        if first.contains(&at) {
            continue;
        }
        if let Some(face) = (0..4).find(|face| faces[*face].distance(*point) > eps) {
            faces[face].outside.push(at);
        }
    }

    let mut next = 0;
    loop {
        // The first face, in the order they were made, with points outside it.
        while next < faces.len() && (!faces[next].alive || faces[next].outside.is_empty()) {
            next += 1;
        }
        if next == faces.len() {
            break;
        }
        let face = next;
        let eye = faces[face]
            .outside
            .iter()
            .copied()
            .fold(None, |best: Option<usize>, at| {
                let far = faces[face].distance(points[at]);
                match best {
                    Some(best) if faces[face].distance(points[best]) >= far => Some(best),
                    _ => Some(at),
                }
            });
        let eye = eye.expect("the face has points outside it");
        let seen = points[eye];

        // The faces the eye sees, found from this one across shared
        // edges, and the edges round them (the horizon).
        let mut visible = vec![face];
        let mut marked: FxHashMap<usize, bool> = FxHashMap::default();
        marked.insert(face, true);
        let mut horizon: Vec<(usize, usize)> = Vec::new();
        let mut at = 0;
        while at < visible.len() {
            let held = visible[at];
            at += 1;
            let corners = faces[held].corners;
            for k in 0..3 {
                let (from, to) = (corners[k], corners[(k + 1) % 3]);
                let Some(&other) = edges.get(&(to, from)) else {
                    continue;
                };
                match marked.get(&other) {
                    Some(true) => {}
                    Some(false) => horizon.push((from, to)),
                    None => {
                        let sees = faces[other].distance(seen) > eps;
                        marked.insert(other, sees);
                        if sees {
                            visible.push(other);
                        } else {
                            horizon.push((from, to));
                        }
                    }
                }
            }
        }
        // Each horizon edge belongs to one visible face, which is gone
        // through once, so each is found once.
        let kept = horizon;

        // The points the visible faces held, to share out among the new.
        let mut loose: Vec<usize> = Vec::new();
        for &held in &visible {
            loose.append(&mut faces[held].outside);
            faces[held].alive = false;
            let corners = faces[held].corners;
            for k in 0..3 {
                let edge = (corners[k], corners[(k + 1) % 3]);
                if edges.get(&edge) == Some(&held) {
                    edges.remove(&edge);
                }
            }
        }
        loose.sort_unstable();
        let made: Vec<usize> = kept
            .iter()
            .map(|&(from, to)| add(&mut faces, &mut edges, [from, to, eye]))
            .collect();
        for point in loose {
            if point == eye {
                continue;
            }
            if let Some(&face) = made
                .iter()
                .find(|face| faces[**face].distance(points[point]) > eps)
            {
                faces[face].outside.push(point);
            }
        }
        // Only the new faces, which come after `next`, have been given
        // points, so the scan goes on from where it was.
    }

    // The hull's corners in the order its faces first use them.
    let mut index: Vec<Option<u32>> = vec![None; points.len()];
    let mut out = Hull {
        points: Vec::new(),
        triangles: Vec::new(),
    };
    for face in faces.iter().filter(|face| face.alive) {
        let corners = face.corners.map(|at| {
            *index[at].get_or_insert_with(|| {
                out.points.push(points[at]);
                (out.points.len() - 1) as u32
            })
        });
        out.triangles.push(corners);
    }
    Some(out)
}

impl Hull {
    /// The volume it encloses, in mm³.
    pub fn volume(&self) -> f64 {
        self.triangles
            .iter()
            .map(|t| {
                let [a, b, c] = t.map(|at| self.points[at as usize]);
                dot(a, cross(b, c))
            })
            .sum::<f64>()
            / 6.0
    }

    /// Each triangle's neighbour across each of its edges (edge `k` runs
    /// from corner `k` to corner `k + 1`), or none where no triangle shares
    /// the edge the other way round.
    pub fn neighbours(&self) -> Vec<[Option<usize>; 3]> {
        let mut edges: FxHashMap<(u32, u32), usize> = FxHashMap::default();
        for (at, t) in self.triangles.iter().enumerate() {
            for k in 0..3 {
                edges.insert((t[k], t[(k + 1) % 3]), at);
            }
        }
        self.triangles
            .iter()
            .map(|t| std::array::from_fn(|k| edges.get(&(t[(k + 1) % 3], t[k])).copied()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_points() -> Vec<[f64; 3]> {
        (0..8)
            .map(|i| std::array::from_fn(|axis| if i >> axis & 1 == 1 { 10.0 } else { 0.0 }))
            .collect()
    }

    /// Every point lies inside or on every face, and every face is closed
    /// off by three neighbours.
    fn check(hull: &Hull, points: &[[f64; 3]]) {
        let size = 1e-6 * 100.0;
        for t in &hull.triangles {
            let face = Face::new(&hull.points, t.map(|at| at as usize));
            for point in points {
                assert!(face.distance(*point) <= size, "a point lies outside a face");
            }
        }
        for neighbours in hull.neighbours() {
            assert!(
                neighbours.iter().all(Option::is_some),
                "the hull is not closed"
            );
        }
    }

    #[test]
    fn a_cube_is_its_own_hull() {
        let points = cube_points();
        let hull = convex_hull(&points).unwrap();
        assert_eq!(hull.points.len(), 8);
        assert_eq!(hull.triangles.len(), 12);
        assert!((hull.volume() - 1000.0).abs() < 1e-9);
        check(&hull, &points);
    }

    #[test]
    fn points_inside_and_on_faces_are_left_out() {
        let mut points = cube_points();
        // The middle, the middle of each face, the middle of an edge.
        points.push([5.0, 5.0, 5.0]);
        for axis in 0..3 {
            for side in [0.0, 10.0] {
                let mut p = [5.0; 3];
                p[axis] = side;
                points.push(p);
            }
        }
        points.push([5.0, 0.0, 0.0]);
        let hull = convex_hull(&points).unwrap();
        assert_eq!(hull.points.len(), 8);
        assert!((hull.volume() - 1000.0).abs() < 1e-9);
        check(&hull, &points);
    }

    #[test]
    fn an_octahedron_and_a_tetrahedron() {
        let octahedron = [
            [1.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
        ];
        let hull = convex_hull(&octahedron).unwrap();
        assert_eq!(hull.triangles.len(), 8);
        assert!((hull.volume() - 4.0 / 3.0).abs() < 1e-12);
        let tetrahedron = [
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let hull = convex_hull(&tetrahedron).unwrap();
        assert_eq!(hull.triangles.len(), 4);
        assert!((hull.volume() - 1.0 / 6.0).abs() < 1e-12);
    }

    #[test]
    fn points_on_a_sphere_are_all_corners() {
        // A spiral of points over a sphere: every one is on the hull.
        let n = 200;
        let points: Vec<[f64; 3]> = (0..n)
            .map(|i| {
                let z = 1.0 - 2.0 * (i as f64 + 0.5) / n as f64;
                let r = (1.0 - z * z).sqrt();
                let a = i as f64 * 2.399_963_229_728_653;
                [10.0 * r * a.cos(), 10.0 * r * a.sin(), 10.0 * z]
            })
            .collect();
        let hull = convex_hull(&points).unwrap();
        assert_eq!(hull.points.len(), n);
        // Euler: a closed hull of triangles has 2V - 4 faces.
        assert_eq!(hull.triangles.len(), 2 * n - 4);
        let sphere = 4.0 / 3.0 * std::f64::consts::PI * 1000.0;
        assert!(hull.volume() < sphere && hull.volume() > 0.95 * sphere);
        check(&hull, &points);
    }

    #[test]
    fn a_cloud_is_held_and_the_answer_is_the_same_every_time() {
        // Points from a fixed sequence, the same on every machine.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 11) as f64 / (1u64 << 53) as f64 * 100.0
        };
        let points: Vec<[f64; 3]> = (0..2000).map(|_| [next(), next(), next()]).collect();
        let hull = convex_hull(&points).unwrap();
        check(&hull, &points);
        assert_eq!(hull.triangles.len(), 2 * hull.points.len() - 4);
        assert_eq!(convex_hull(&points), Some(hull));
    }

    #[test]
    fn flat_and_straight_sets_have_none() {
        assert_eq!(convex_hull(&[[0.0; 3]; 3]), None);
        let flat: Vec<[f64; 3]> = (0..10).map(|i| [i as f64, (i * i) as f64, 2.0]).collect();
        assert_eq!(convex_hull(&flat), None);
        let line: Vec<[f64; 3]> = (0..10).map(|i| [i as f64, 2.0 * i as f64, 0.0]).collect();
        assert_eq!(convex_hull(&line), None);
    }
}
