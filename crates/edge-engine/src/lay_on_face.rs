//! The faces an object can be laid on, and the turn that lays it on one:
//! the reference's "Lay on face" (libslic3r/LayOnFace.cpp,
//! Gizmos/GLGizmoFlatten.cpp `update_planes`, Selection.cpp
//! `flattening_rotate`).
//!
//! An object can only rest on its convex hull, so the faces offered are the
//! hull's: its triangles grouped where they are coplanar, each group's
//! outline taken round, and the small ones dropped by the reference's rule
//! for its gizmo, measured at the size the object is printed at.
//!
//! Matrices here are of the usual kind (a point is a column, `m[row][col]`);
//! [`linear_of`] makes one of a [`Transform`].

use edge_mesh::{Transform, hull::Hull};

/// A 3 by 3 matrix, `m[row][col]`, applied to a column.
pub type Mat3 = [[f64; 3]; 3];

/// One face to lay an object on.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// Outward, of unit length, in the object's own coordinates.
    pub normal: [f64; 3],
    /// In mm², at the size the object is printed at.
    pub area: f64,
    /// Its outline, convex, in the object's own coordinates.
    pub outline: Vec<[f64; 3]>,
    /// What is drawn over the object for it, in the object's own
    /// coordinates: the outline drawn in a tenth of the way to its middle,
    /// its corners rounded, and lifted a little off the surface, as the
    /// reference draws it (`update_planes`).
    pub drawn: Vec<[f64; 3]>,
}

/// Which faces are offered (LayOnFace.hpp `LayOnFaceFilter`), measured at
/// the printed size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Filter {
    /// mm²: a face at least this large is always offered.
    pub min_area: f64,
    /// mm: a face of one triangle with a shorter side is dropped.
    pub min_side: f64,
    /// A face whose outline has a sharper inner angle is dropped.
    pub min_angle_deg: f64,
    /// When fewer than this many reach `min_area`, the largest of the
    /// smaller ones down to `floor_area` make up the number.
    pub min_count: usize,
    pub floor_area: f64,
}

impl Filter {
    /// The gizmo's own (`LayOnFaceFilter::for_gizmo`), for something whose
    /// box has a diagonal of `size` mm: at least the 12 largest faces down
    /// to max(0.01 mm², 1e-4 × size²), and a least side of min(1 mm, 5% of
    /// the size).
    pub fn for_gizmo(size: f64) -> Self {
        Self {
            min_area: 5.0,
            min_side: (0.05 * size).min(1.0),
            min_angle_deg: 1.0,
            min_count: 12,
            floor_area: 5f64.min((1e-4 * size * size).max(0.01)),
        }
    }
}

/// The linear part of a transform, as a matrix of the usual kind.
pub fn linear_of(transform: &Transform) -> Mat3 {
    std::array::from_fn(|row| std::array::from_fn(|col| transform.rows[col][row]))
}

/// The transform a matrix of the usual kind stands for, moving nothing.
pub fn transform_of(m: &Mat3) -> Transform {
    let mut out = Transform::IDENTITY;
    for (row, held) in out.rows.iter_mut().take(3).enumerate() {
        *held = std::array::from_fn(|col| m[col][row]);
    }
    out
}

fn apply(m: &Mat3, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn transpose(m: &Mat3) -> Mat3 {
    std::array::from_fn(|r| std::array::from_fn(|c| m[c][r]))
}

fn inverse(m: &Mat3) -> Option<Mat3> {
    let c = |r: usize, k: usize| {
        let (r1, r2) = ((r + 1) % 3, (r + 2) % 3);
        let (k1, k2) = ((k + 1) % 3, (k + 2) % 3);
        m[r1][k1] * m[r2][k2] - m[r1][k2] * m[r2][k1]
    };
    let det = m[0][0] * c(0, 0) + m[0][1] * c(0, 1) + m[0][2] * c(0, 2);
    if det.abs() < 1e-300 || !det.is_finite() {
        return None;
    }
    Some(std::array::from_fn(|r| {
        std::array::from_fn(|k| c(k, r) / det)
    }))
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

fn unit(a: [f64; 3]) -> [f64; 3] {
    let size = dot(a, a).sqrt();
    if size > 0.0 { a.map(|v| v / size) } else { a }
}

/// The turn that takes the direction `from` to `to`, as Eigen's
/// `Quaternion::setFromTwoVectors` makes it. Where the two are opposite
/// Eigen turns about an axis it takes from a singular value
/// decomposition; this turns about X, or about Y when `from` lies along X.
pub fn turn_between(from: [f64; 3], to: [f64; 3]) -> Mat3 {
    let (v0, v1) = (unit(from), unit(to));
    let c = dot(v1, v0);
    let (w, x, y, z) = if c < -1.0 + 1e-12 {
        // X, or Y, made square to `from`.
        let along = if v0[0].abs() < 0.9 {
            [1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0]
        };
        let axis = unit(sub(along, v0.map(|v| v * dot(v0, along))));
        let w2 = (1.0 + c.max(-1.0)) * 0.5;
        let s = (1.0 - w2).sqrt();
        (w2.sqrt(), axis[0] * s, axis[1] * s, axis[2] * s)
    } else {
        let axis = cross(v0, v1);
        let s = ((1.0 + c) * 2.0).sqrt();
        (s * 0.5, axis[0] / s, axis[1] / s, axis[2] / s)
    };
    // As Eigen's toRotationMatrix, of the quaternion made unit length.
    let n = (w * w + x * x + y * y + z * z).sqrt();
    let (w, x, y, z) = (w / n, x / n, y / n, z / n);
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ]
}

/// The convex outline of points in the plane, by their x and y, as the
/// reference takes it (Geometry/ConvexHull.cpp `convex_hull(Pointf3s)`):
/// the points scaled to a box of one unit and compared on a grid of a
/// millionth of it, so points that differ by float noise count as one and
/// those on a line between two others are left out. Anticlockwise.
fn outline_2d(points: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for p in points {
        for axis in 0..2 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }
    let sf = (1.0 / (high[0] - low[0])).min(1.0 / (high[1] - low[1]));
    if !sf.is_finite() || points.len() < 3 {
        return Vec::new();
    }
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    // Point::new_scale of the point scaled by `sf` (SCALING_FACTOR 1e-6).
    let grid = |p: &[f64; 3]| -> (i64, i64) {
        (
            (p[0] * sf / 1e-6).round() as i64,
            (p[1] * sf / 1e-6).round() as i64,
        )
    };
    let ccw = |p: &[f64; 3], a: &[f64; 3], b: &[f64; 3]| {
        let (p, a, b) = (grid(p), grid(a), grid(b));
        let det =
            (a.0 - p.0) as i128 * (b.1 - p.1) as i128 - (a.1 - p.1) as i128 * (b.0 - p.0) as i128;
        det > 0
    };
    let n = sorted.len();
    let mut hull: Vec<[f64; 3]> = Vec::with_capacity(2 * n);
    for p in &sorted {
        while hull.len() >= 2 && !ccw(p, &hull[hull.len() - 2], &hull[hull.len() - 1]) {
            hull.pop();
        }
        hull.push(*p);
    }
    let t = hull.len() + 1;
    for p in sorted.iter().rev().skip(1) {
        while hull.len() >= t && !ccw(p, &hull[hull.len() - 2], &hull[hull.len() - 1]) {
            hull.pop();
        }
        hull.push(*p);
    }
    hull.pop();
    hull
}

/// The outline drawn smaller and with its corners rounded, as the
/// reference draws a face (GLGizmoFlatten.cpp `update_planes`), in the
/// plane's own frame, where it lies flat.
fn rounded(polygon: &[[f64; 3]], lift: f64) -> Vec<[f64; 3]> {
    let n = polygon.len();
    if n < 3 {
        return polygon.to_vec();
    }
    let mut centroid = [0.0; 3];
    for p in polygon {
        for axis in 0..3 {
            centroid[axis] += p[axis] / n as f64;
        }
    }
    let polygon: Vec<[f64; 3]> = polygon
        .iter()
        .map(|p| std::array::from_fn(|axis| 0.9 * p[axis] + 0.1 * centroid[axis]))
        .collect();
    let mix = |a: [f64; 3], b: [f64; 3], r: f64| -> [f64; 3] {
        std::array::from_fn(|axis| r * a[axis] + (1.0 - r) * b[axis])
    };
    const K: usize = 10;
    const AGGRESSIVITY: f64 = 0.2;
    let mut out = vec![[0.0; 3]; 2 * K * n];
    let mut neighbours: Vec<(usize, usize)> = Vec::with_capacity(n);
    for (j, p) in polygon.iter().enumerate() {
        out[j * 2 * K] = *p;
        let before = if j == 0 {
            (n - 1) * 2 * K + K
        } else {
            j * 2 * K - K
        };
        neighbours.push((before, j * 2 * K + K));
    }
    for i in 0..K {
        for j in 0..n {
            if i == 0 {
                let next = if j == n - 1 { 0 } else { (j + 1) * 2 * K };
                out[j * 2 * K + K] = mix(out[j * 2 * K], out[next], 0.5);
            } else {
                let r = 0.2 + 0.3 / (K - 1) as f64 * i as f64;
                let (first, second) = neighbours[j];
                out[first] = mix(out[j * 2 * K], out[first - 1], r);
                out[second] = mix(out[j * 2 * K], out[second + 1], r);
            }
        }
        for j in 0..n {
            let (first, second) = neighbours[j];
            let middle = mix(out[first], out[second], 0.5);
            out[2 * K * j] = mix(out[2 * K * j], middle, 1.0 - AGGRESSIVITY);
        }
        for held in &mut neighbours {
            held.0 += 1;
            held.1 -= 1;
        }
    }
    for p in &mut out {
        p[2] += lift;
    }
    out
}

/// The faces of `hull` (in the object's own coordinates) to lay the object
/// on, largest first, when `linear` sizes and turns the object as it is
/// printed (its copy's transform without the move). `size` is the
/// diagonal of the box round the object at that size, which the drawn
/// outline is lifted by a hundredth of (at most 0.1 mm).
pub fn faces(hull: &Hull, linear: &Mat3, filter: &Filter, size: f64) -> Vec<Face> {
    let count = hull.triangles.len();
    let normals: Vec<[f64; 3]> = hull
        .triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|at| hull.points[at as usize]);
            unit(cross(sub(b, a), sub(c, a)))
        })
        .collect();
    let neighbours = hull.neighbours();
    let minimal_area = if filter.min_count > 0 {
        filter.floor_area.min(filter.min_area)
    } else {
        filter.min_area
    };

    // The triangles of each plane: from the first one not yet taken, those
    // reached across edges with the same normal, to a thousandth.
    let mut groups: Vec<([f64; 3], Vec<[f64; 3]>)> = Vec::new();
    let mut visited = vec![false; count];
    let mut queue: Vec<usize> = Vec::new();
    let mut first = 0;
    loop {
        while first < count && visited[first] {
            first += 1;
        }
        if first == count {
            break;
        }
        let normal = normals[first];
        visited[first] = true;
        queue.push(first);
        let mut points = Vec::new();
        while let Some(facet) = queue.pop() {
            let this = normals[facet];
            if (0..3).all(|axis| (this[axis] - normal[axis]).abs() < 0.001) {
                for &corner in &hull.triangles[facet] {
                    points.push(hull.points[corner as usize]);
                }
                visited[facet] = true;
                for next in neighbours[facet].into_iter().flatten() {
                    if !visited[next] {
                        queue.push(next);
                    }
                }
            }
        }
        // At the printed size.
        let points: Vec<[f64; 3]> = points.iter().map(|p| apply(linear, *p)).collect();
        // A very small single triangle is no face to rest on.
        let side = |a: [f64; 3], b: [f64; 3]| dot(sub(a, b), sub(a, b)).sqrt();
        if points.len() == 3
            && (side(points[0], points[1]) < filter.min_side
                || side(points[0], points[2]) < filter.min_side
                || side(points[1], points[2]) < filter.min_side)
        {
            continue;
        }
        groups.push((normal, points));
    }

    let Some(back) = inverse(linear) else {
        return Vec::new();
    };
    let normal_matrix = transpose(&back);
    let lift = (0.01 * size).min(0.1);
    let threshold = (filter.min_angle_deg * std::f64::consts::PI / 180.0).cos();
    let mut out: Vec<Face> = Vec::new();
    for (normal, points) in groups {
        let turn = turn_between(apply(&normal_matrix, normal), [0.0, 0.0, 1.0]);
        let flat: Vec<[f64; 3]> = points.iter().map(|p| apply(&turn, *p)).collect();
        let polygon = outline_2d(&flat);
        if polygon.len() < 3 {
            continue;
        }
        let mut area = 0.0;
        for i in 0..polygon.len() {
            let next = polygon[(i + 1) % polygon.len()];
            area += polygon[i][0] * next[1] - next[0] * polygon[i][1];
        }
        let area = 0.5 * area.abs();
        if area < minimal_area {
            continue;
        }
        let sharp = (0..polygon.len()).any(|i| {
            let before = polygon[(i + polygon.len() - 1) % polygon.len()];
            let next = polygon[(i + 1) % polygon.len()];
            let curr = polygon[i];
            dot(unit(sub(before, curr)), unit(sub(next, curr))) > threshold
        });
        if sharp {
            continue;
        }
        // Back to the object's own coordinates.
        let to_object = |p: &[f64; 3]| apply(&back, apply(&transpose(&turn), *p));
        out.push(Face {
            normal,
            area,
            outline: polygon.iter().map(to_object).collect(),
            drawn: rounded(&polygon, lift).iter().map(to_object).collect(),
        });
    }
    // Largest first; of equal ones, in the order they were found.
    out.sort_by(|a, b| b.area.total_cmp(&a.area));
    if minimal_area < filter.min_area {
        let keep = out
            .iter()
            .take_while(|face| face.area >= filter.min_area)
            .count();
        out.truncate(keep.max(filter.min_count.min(out.len())));
    }
    out
}

/// The turn that lays an object on the face with outward `normal` (in its
/// own coordinates) when `linear` turns and sizes it: the face's normal as
/// printed is turned to point straight down (Selection.cpp
/// `flattening_rotate`).
pub fn turn_down(linear: &Mat3, normal: [f64; 3]) -> Option<Mat3> {
    let normal_matrix = transpose(&inverse(linear)?);
    let printed = apply(&normal_matrix, normal);
    if dot(printed, printed) == 0.0 || !printed.iter().all(|v| v.is_finite()) {
        return None;
    }
    Some(turn_between(printed, [0.0, 0.0, -1.0]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::hull::convex_hull;

    const IDENTITY: Mat3 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];

    fn boxed(size: [f64; 3]) -> Hull {
        let points: Vec<[f64; 3]> = (0..8)
            .map(|i| std::array::from_fn(|axis| if i >> axis & 1 == 1 { size[axis] } else { 0.0 }))
            .collect();
        convex_hull(&points).unwrap()
    }

    #[test]
    fn a_box_has_six_faces_the_largest_first() {
        let hull = boxed([30.0, 20.0, 10.0]);
        let faces = faces(&hull, &IDENTITY, &Filter::for_gizmo(37.4), 37.4);
        assert_eq!(faces.len(), 6);
        let areas: Vec<f64> = faces
            .iter()
            .map(|face| (face.area * 1e6).round() / 1e6)
            .collect();
        assert_eq!(areas, [600.0, 600.0, 300.0, 300.0, 200.0, 200.0]);
        for face in &faces {
            assert_eq!(face.outline.len(), 4);
            // Every corner of the outline is on the face's plane.
            let level = dot(face.normal, face.outline[0]);
            for p in &face.outline {
                assert!((dot(face.normal, *p) - level).abs() < 1e-9);
            }
            // Drawn just outside it, a little smaller.
            for p in &face.drawn {
                let off = dot(face.normal, *p) - level;
                assert!(off > 0.0 && off <= 0.1 + 1e-9);
            }
        }
        // The two largest are the top and the bottom.
        let mut big: Vec<[f64; 3]> = faces[..2].iter().map(|face| face.normal).collect();
        big.sort_by(|a, b| a[2].total_cmp(&b[2]));
        assert_eq!(big, [[0.0, 0.0, -1.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn faces_are_measured_at_the_printed_size() {
        // A box 2 x 2 x 2 has faces of 4 mm², under 5 mm²; scaled up five
        // times, 100 mm². Either way all six are offered (the gizmo makes
        // up twelve from smaller ones), with their areas at that size.
        let hull = boxed([2.0, 2.0, 2.0]);
        let small = faces(&hull, &IDENTITY, &Filter::for_gizmo(3.46), 3.46);
        assert_eq!(small.len(), 6);
        assert!((small[0].area - 4.0).abs() < 1e-9);
        let five = [[5.0, 0.0, 0.0], [0.0, 5.0, 0.0], [0.0, 0.0, 5.0]];
        let big = faces(&hull, &five, &Filter::for_gizmo(17.3), 17.3);
        assert!((big[0].area - 100.0).abs() < 1e-9);
        // With the CLI's strict limits the small box offers none.
        let strict = Filter {
            min_area: 5.0,
            min_side: 1.0,
            min_angle_deg: 1.0,
            min_count: 0,
            floor_area: 0.0,
        };
        assert!(faces(&hull, &IDENTITY, &strict, 3.46).is_empty());
    }

    #[test]
    fn small_faces_are_dropped_past_the_twelve_largest() {
        // A prism of 40 sides: two caps and 40 thin sides. The caps and the
        // sides of 5 mm² or more stay.
        let mut points = Vec::new();
        for i in 0..40 {
            let a = i as f64 * std::f64::consts::TAU / 40.0;
            for z in [0.0, 1.0] {
                points.push([10.0 * a.cos(), 10.0 * a.sin(), z]);
            }
        }
        let hull = convex_hull(&points).unwrap();
        let faces = faces(&hull, &IDENTITY, &Filter::for_gizmo(20.1), 20.1);
        // Each side is 1.57 mm by 1 mm: under 5 mm², so only enough of
        // them to make twelve with the caps.
        assert_eq!(faces.len(), 12);
        assert!(faces[0].area > 300.0 && faces[1].area > 300.0);
    }

    #[test]
    fn a_turn_down_points_the_face_at_the_bed() {
        let turned = turn_down(&IDENTITY, [1.0, 0.0, 0.0]).unwrap();
        let n = apply(&turned, [1.0, 0.0, 0.0]);
        assert!((n[2] + 1.0).abs() < 1e-12 && n[0].abs() < 1e-12);
        // Already down: nothing to do.
        let none = turn_down(&IDENTITY, [0.0, 0.0, -1.0]).unwrap();
        for r in 0..3 {
            for c in 0..3 {
                assert!((none[r][c] - IDENTITY[r][c]).abs() < 1e-12);
            }
        }
        // Straight up: half a turn about X.
        let up = turn_down(&IDENTITY, [0.0, 0.0, 1.0]).unwrap();
        let n = apply(&up, [0.0, 0.0, 1.0]);
        assert!((n[2] + 1.0).abs() < 1e-12);
        assert!((apply(&up, [1.0, 0.0, 0.0])[0] - 1.0).abs() < 1e-12);
        // Under a squash, the normal goes as normals go (the inverse
        // transpose), not as points go.
        let squash = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 0.0, 1.0]];
        let normal = unit([1.0, 0.0, 1.0]);
        let turned = turn_down(&squash, normal).unwrap();
        let printed = apply(&transpose(&inverse(&squash).unwrap()), normal);
        let n = unit(apply(&turned, printed));
        assert!((n[2] + 1.0).abs() < 1e-12);
    }

    #[test]
    fn the_rounded_outline_has_twenty_points_a_corner() {
        let square = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [10.0, 10.0, 0.0],
            [0.0, 10.0, 0.0],
        ];
        let drawn = rounded(&square, 0.1);
        assert_eq!(drawn.len(), 80);
        for p in &drawn {
            assert!(p[0] > 0.4 && p[0] < 9.6 && p[1] > 0.4 && p[1] < 9.6);
            assert!((p[2] - 0.1).abs() < 1e-12);
        }
    }

    #[test]
    fn matrices_and_transforms_agree() {
        let mut t = Transform::IDENTITY;
        t.rows[0] = [0.0, 1.0, 0.0];
        t.rows[1] = [-1.0, 0.0, 0.0];
        let m = linear_of(&t);
        assert_eq!(apply(&m, [1.0, 0.0, 0.0]), t.apply([1.0, 0.0, 0.0]));
        assert_eq!(transform_of(&m), t);
    }
}
