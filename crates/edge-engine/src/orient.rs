//! Auto orient: which way up a mesh prints best, ported from the
//! reference's `libslic3r/Orient.cpp` (`AutoOrienter`) step for step, with
//! its cost function, its two sets of weights and the way it finds and
//! merges the directions it tries.
//!
//! A direction tried is a direction "up" (`orientation` in the reference).
//! For each the mesh's corners are measured along it, and from that the
//! area resting on the bed, the area of its convex hull resting there, the
//! area hanging over, and the area of faces that are nearly flat but not
//! quite; [`Params`] weighs them into one number, the lowest wins.
//!
//! What is the reference's and what is not:
//!
//! - Every comparison is made in `f32`, as the reference makes it, with
//!   its thresholds as it rounds them. The sums of areas are added in
//!   `f64` in face order and then rounded, where the reference adds `f32`s
//!   in the order its vector unit takes them: the same to the last bit or
//!   two.
//! - The reference keeps the directions it found and their costs in hash
//!   maps and sorts them unstably, so among directions of equal area (or
//!   equal cost) which comes first is the hash's order. Here equals keep
//!   the order they were found in: the mesh's own direction first, then
//!   the directions of the most area, then the fixed ones. Only a tie
//!   decides it, and any of the tied directions is as good by the
//!   reference's measure.
//! - The reference's last check, that a direction tied with the best which
//!   leaves the mesh as it stands is taken, looks at every result but the
//!   last (`i < results_vector.size()-1`); here it looks at all of them.
//! - Faces marked as the outer look of a part (`eExteriorAppearance`, set
//!   only by the reference's face detector tool, which is not kept in a
//!   file) weigh three times more as overhang there; no face is marked
//!   here, so each weighs once.
//! - Each direction is measured on its own and the results gathered in
//!   order, so any number of threads gives the same answer.

use rayon::prelude::*;

use crate::lay_on_face::Mat3;

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// The weights and thresholds of the cost (Orient.hpp `OrientParams`,
/// `OrientParamsArea`). The names are the reference's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub tar_a: f32,
    pub tar_b: f32,
    pub relative_f: f32,
    pub contour_f: f32,
    pub bottom_f: f32,
    pub bottom_hull_f: f32,
    pub tar_c: f32,
    pub tar_d: f32,
    pub tar_e: f32,
    pub first_lay_h: f32,
    /// The cosine past which a face hangs over: `cos(π − angle)` for the
    /// support threshold angle.
    pub ascent: f32,
    pub laf_max: f32,
    pub laf_min: f32,
    pub tar_laf: f32,
    pub tar_proj_area: f32,
    pub bottom_min: f32,
    pub apperance_face_supp: f32,
    pub use_low_angle_face: bool,
    /// The other cost (`min_volume`), weighing overhangs by their height;
    /// neither the application nor the command line uses it.
    pub min_volume: bool,
}

impl Params {
    /// What the reference's command line orients with (`--orient 1`,
    /// Orient.cpp `orient(ModelObject*)`): `OrientParams` as they are,
    /// overhang past 60° (`ASCENT` −0.5).
    // The reference's literals as it writes them, rounded to `f32` alike.
    #[allow(clippy::excessive_precision)]
    pub fn command_line() -> Self {
        Self {
            tar_a: 0.01,
            tar_b: 0.177,
            relative_f: 6.610621027964314,
            contour_f: 0.23228623269775997,
            bottom_f: 1.167152017941474,
            bottom_hull_f: 0.1,
            tar_c: 0.24308070476924726,
            tar_d: 0.6284515508160871,
            tar_e: 0.0,
            first_lay_h: 0.2,
            ascent: -0.5,
            laf_max: 0.999,
            laf_min: 0.9703,
            tar_laf: 0.01,
            tar_proj_area: 0.1,
            bottom_min: 0.1,
            apperance_face_supp: 3.0,
            use_low_angle_face: true,
            min_volume: false,
        }
    }

    /// What the application orients with (Jobs/OrientJob.cpp `process`):
    /// its orient settings have "Optimize support interface area"
    /// (`min_area`) on, which no window offers to change, so
    /// `OrientParamsArea` is copied over `OrientParams` and `min_volume`
    /// is off; the overhang is past the object's support threshold angle
    /// (`get_orient_mesh`, `AutoOrienter::AutoOrienter`).
    pub fn application(support_threshold_angle: f64) -> Self {
        Self {
            tar_a: 0.015,
            tar_b: 0.177,
            relative_f: 20.0,
            contour_f: 0.5,
            bottom_f: 2.5,
            bottom_hull_f: 0.1,
            tar_c: 0.1,
            tar_d: 1.0,
            tar_e: 0.0115,
            first_lay_h: 0.2,
            ascent: ascent(support_threshold_angle),
            laf_max: 0.999,
            laf_min: 0.97,
            tar_laf: 0.001,
            tar_proj_area: 0.1,
            bottom_min: 0.1,
            apperance_face_supp: 3.0,
            use_low_angle_face: true,
            min_volume: false,
        }
    }
}

/// `params.ASCENT = cos(PI - overhang_angle * PI / 180)` (the
/// `AutoOrienter` constructor), in `double` and kept as a `float`.
pub fn ascent(overhang_angle: f64) -> f32 {
    // libslic3r's PI.
    const PI: f64 = std::f64::consts::PI;
    (PI - overhang_angle * PI / 180.0).cos() as f32
}

/// What a direction costs (Orient.cpp `CostItems`; those it never fills,
/// the volume, total area, radius and height ratio, are left out).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Costs {
    pub overhang: f32,
    pub bottom: f32,
    pub bottom_hull: f32,
    pub contour: f32,
    pub area_laf: f32,
    /// Never measured by the reference: always 0.
    pub area_projected: f32,
    pub unprintability: f32,
}

/// One direction tried, and what it cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tried {
    /// Up, as the reference measures along it.
    pub up: [f32; 3],
    pub costs: Costs,
}

/// The answer for one mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Orientation {
    /// The direction that is to point up.
    pub up: [f64; 3],
    /// The turn that takes it there (Geometry.cpp
    /// `rotation_from_two_vectors`), a matrix applied to a column.
    pub rotation: Mat3,
    /// Every direction tried, in the order tried.
    pub tried: Vec<Tried>,
}

/// A mesh as the orienter takes it: corners and triangles, wound
/// counter-clockwise seen from outside.
#[derive(Clone, Copy, Debug)]
pub struct Faces<'a> {
    pub vertices: &'a [[f32; 3]],
    pub triangles: &'a [[u32; 3]],
}

/// The mesh with what the reference works out of it once
/// (`AutoOrienter::preprocess`).
struct Prepared<'a> {
    mesh: Faces<'a>,
    hull: Faces<'a>,
    normals: Vec<[f32; 3]>,
    areas: Vec<f32>,
    hull_normals: Vec<[f32; 3]>,
    hull_areas: Vec<f32>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Eigen's `cross` as the reference is built: each `a * b - c * d` with
/// the first product fused into the subtraction (clang contracts it on
/// arm64 and x86-64 with FMA alike), which is what gives a face lying
/// square to an axis a normal a hair off it there, and so a group of its
/// own.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1].mul_add(b[2], -(a[2] * b[1])),
        a[2].mul_add(b[0], -(a[0] * b[2])),
        a[0].mul_add(b[1], -(a[1] * b[0])),
    ]
}

/// Eigen's sum of three, unrolled as `x0 + (x1 + x2)`.
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + (a[1] * b[1] + a[2] * b[2])
}

/// Eigen's `normalized()`: unchanged when of no length.
fn normalized(a: [f32; 3]) -> [f32; 3] {
    let n2 = dot(a, a);
    if n2 > 0.0 {
        let n = n2.sqrt();
        a.map(|v| v / n)
    } else {
        a
    }
}

fn corners(mesh: &Faces<'_>, face: usize) -> [[f32; 3]; 3] {
    mesh.triangles[face].map(|v| mesh.vertices[v as usize])
}

/// TriangleMesh.hpp `face_normal_normalized`: `(v1 − v0) × (v2 − v1)`,
/// normalized twice.
fn face_normal(v: [[f32; 3]; 3]) -> [f32; 3] {
    normalized(normalized(cross(sub(v[1], v[0]), sub(v[2], v[1]))))
}

/// admesh's `facet_area`: `|(v0 − v1) × (v0 − v2)| / 2`.
fn facet_area(v: [[f32; 3]; 3]) -> f32 {
    let c = cross(sub(v[0], v[1]), sub(v[0], v[2]));
    dot(c, c).sqrt().abs() / 2.0
}

/// `quantize_vec3f`: each part floored to a thousandth.
fn quantize(n: [f32; 3]) -> [f32; 3] {
    n.map(|v| (v * 1000.0).floor() / 1000.0)
}

/// Eigen's `isApprox` for two vectors: `‖a − b‖² ≤ tol² · min(‖a‖², ‖b‖²)`.
fn is_approx(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    let d = sub(a, b);
    dot(d, d) <= tol * tol * dot(a, a).min(dot(b, b))
}

impl<'a> Prepared<'a> {
    fn new(mesh: Faces<'a>, hull: Faces<'a>) -> Self {
        let measure = |mesh: &Faces<'_>| -> (Vec<[f32; 3]>, Vec<f32>) {
            (0..mesh.triangles.len())
                .map(|face| {
                    let v = corners(mesh, face);
                    (face_normal(v), facet_area(v))
                })
                .unzip()
        };
        let (normals, areas) = measure(&mesh);
        let (hull_normals, hull_areas) = measure(&hull);
        Self {
            mesh,
            hull,
            normals,
            areas,
            hull_normals,
            hull_areas,
        }
    }

    /// `area_cumulation_accurate`: the faces grouped by their normal
    /// floored to a thousandth, the groups of most area first, each
    /// standing for the normal of its largest face (the first such).
    fn most_area(normals: &[[f32; 3]], areas: &[f32], count: usize) -> Vec<[f32; 3]> {
        // Each group: its key, its total area, its largest face's area
        // and normal. Groups are kept in the order first met, which is
        // what equals keep after the sort (see the module's notes).
        struct Group {
            total: f32,
            largest: f32,
            normal: [f32; 3],
        }
        let mut index: std::collections::HashMap<[u32; 3], usize> = Default::default();
        let mut groups: Vec<Group> = Vec::new();
        for (normal, &area) in normals.iter().zip(areas) {
            // Equal as floats is equal as bits but for ±0, which the
            // reference's map takes as one key.
            let key = quantize(*normal).map(|v| (v + 0.0).to_bits());
            let at = *index.entry(key).or_insert_with(|| {
                groups.push(Group {
                    total: 0.0,
                    largest: 0.0,
                    normal: [0.0; 3],
                });
                groups.len() - 1
            });
            let group = &mut groups[at];
            group.total += area;
            if area > group.largest {
                group.normal = *normal;
                group.largest = area;
            }
        }
        let mut order: Vec<usize> = (0..groups.len()).collect();
        order.sort_by(|a, b| groups[*b].total.total_cmp(&groups[*a].total));
        order
            .into_iter()
            .take(count)
            .map(|at| groups[at].normal)
            .collect()
    }

    /// The directions to try (`process` up to the costs): the mesh as it
    /// stands, the 10 of most area among its faces and the 14 among its
    /// hull's (`area_cumulation_accurate`), the 18 fixed ones
    /// (`add_supplements`), with what repeats taken out
    /// (`remove_duplicates`). Each is a face's normal, which is to point
    /// down.
    fn candidates(&self) -> Vec<[f32; 3]> {
        let mut out: Vec<[f32; 3]> = vec![[0.0, 0.0, -1.0]];
        out.extend(Self::most_area(&self.normals, &self.areas, 10));
        out.extend(Self::most_area(&self.hull_normals, &self.hull_areas, 14));
        // The reference's 0.70710678, which is this as an `f32`.
        const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
        out.extend([
            [0.0, 0.0, -1.0],
            [H, 0.0, -H],
            [0.0, H, -H],
            [-H, 0.0, -H],
            [0.0, -H, -H],
            [1.0, 0.0, 0.0],
            [H, H, 0.0],
            [0.0, 1.0, 0.0],
            [-H, H, 0.0],
            [-1.0, 0.0, 0.0],
            [-H, -H, 0.0],
            [0.0, -1.0, 0.0],
            [H, -H, 0.0],
            [H, 0.0, H],
            [0.0, H, H],
            [-H, 0.0, H],
            [0.0, -H, H],
            [0.0, 0.0, 1.0],
        ]);
        // `remove_duplicates(tol = 0.0000001)`, the first always kept.
        const TOL: f32 = 0.000_000_1;
        let mut kept: Vec<[f32; 3]> = Vec::with_capacity(out.len());
        for (at, direction) in out.into_iter().enumerate() {
            let repeated = kept.iter().any(|seen| is_approx(*seen, direction, TOL));
            let zero = is_approx(direction, [0.0; 3], TOL);
            if at == 0 || !(repeated || zero) {
                kept.push(direction);
            }
        }
        kept
    }

    /// `project_vertices` and `get_features` for one direction up, then
    /// `target_function`.
    fn cost(&self, up: [f32; 3], params: &Params) -> Costs {
        let projected: Vec<f32> = self.mesh.vertices.iter().map(|v| dot(*v, up)).collect();
        let top = |mesh: &Faces<'_>, projected: &dyn Fn(usize) -> f32, face: usize| {
            let [a, b, c] = mesh.triangles[face].map(|v| projected(v as usize));
            a.max(b).max(c)
        };
        let z_max: Vec<f32> = (0..self.mesh.triangles.len())
            .map(|face| top(&self.mesh, &|v| projected[v], face))
            .collect();
        // `z_projected.minCoeff()`: over the corners of every face, which
        // is every corner that is used.
        let total_min_z = self
            .mesh
            .triangles
            .iter()
            .flatten()
            .map(|v| projected[*v as usize])
            .fold(f32::INFINITY, f32::min);
        // `total_min_z + FIRST_LAY_H - EPSILON`: a float sum, the double
        // EPSILON taken off it, compared as a float.
        let below = |height: f32| ((total_min_z + height) as f64 - EPSILON) as f32;
        let bottom_under = below(params.first_lay_h);
        let bottom_2nd_under = below(params.first_lay_h / 2.0);
        let laf_over = total_min_z + params.first_lay_h;

        // Summed as Eigen sums them: a `select` one by one, in order; a
        // vector it has worked out (the overhangs, the low angle faces)
        // four lanes at a time ([`eigen_sum`]).
        let mut bottom = 0.0f32;
        let mut bottom_2nd = 0.0f32;
        let mut overhangs = vec![0.0f32; z_max.len()];
        let mut lafs = vec![0.0f32; z_max.len()];
        for (face, (&z, &area)) in z_max.iter().zip(&self.areas).enumerate() {
            let in_2nd = z < bottom_2nd_under;
            bottom += if z < bottom_under { area } else { 0.0 };
            bottom_2nd += if in_2nd { area } else { 0.0 };
            let np = dot(self.normals[face], up);
            if np < params.ascent && !in_2nd {
                // No face is marked as the outer look here (see the notes).
                let weighed = area * (0.0 * params.apperance_face_supp + 1.0);
                overhangs[face] = if params.min_volume {
                    let [a, b, c] = self.mesh.triangles[face].map(|v| projected[v as usize]);
                    let height = (a + b + c) / 3.0 - total_min_z;
                    let inner = (np - params.ascent).min(0.0).abs();
                    height * weighed * inner
                } else {
                    weighed.abs()
                };
            }
            let along = np.abs();
            if along < params.laf_max && along > params.laf_min && z > laf_over {
                lafs[face] = area;
            }
        }
        // `sum() * 0.5 + sum()`: a double, kept as a float.
        let bottom = ((bottom as f64) * 0.5 + bottom_2nd as f64) as f32;
        let overhang = eigen_sum(&overhangs);
        let area_laf = eigen_sum(&lafs);

        let hull_projected: Vec<f32> = self.hull.vertices.iter().map(|v| dot(*v, up)).collect();
        let bottom_hull = (0..self.hull.triangles.len())
            .map(|face| {
                if top(&self.hull, &|v| hull_projected[v], face) < bottom_under {
                    self.hull_areas[face]
                } else {
                    0.0
                }
            })
            .fold(0.0f32, |sum, area| sum + area);

        let mut costs = Costs {
            overhang,
            bottom,
            bottom_hull,
            // "the simple way for contour is even better for faces of
            // small bridges"
            contour: 4.0 * bottom.sqrt(),
            area_laf,
            area_projected: 0.0,
            unprintability: 0.0,
        };
        costs.unprintability = target(&costs, params);
        costs
    }
}

/// `target_function`, with each `a * b + c` fused as clang contracts it
/// in the reference's build.
fn target(costs: &Costs, p: &Params) -> f32 {
    let laf = p.tar_laf * costs.area_laf;
    let use_laf = if p.use_low_angle_face { 1.0 } else { 0.0 };
    let over = |overhang: f32| laf.mul_add(use_laf, overhang.mul_add(p.tar_c, p.tar_d));
    let under = p.bottom_hull_f.mul_add(
        costs.bottom_hull,
        p.bottom_f
            .mul_add(costs.bottom, p.contour_f.mul_add(costs.contour, p.tar_d)),
    );
    let mut cost = if p.min_volume {
        let overhang = costs.overhang / 25.0;
        let under = p
            .tar_proj_area
            .mul_add(costs.area_projected, p.tar_e.mul_add(overhang, under));
        p.tar_a
            .mul_add(overhang + p.tar_b, p.relative_f * over(overhang) / under)
    } else {
        let under = p.tar_proj_area.mul_add(costs.area_projected, under);
        p.relative_f * over(costs.overhang) / under
    };
    if costs.bottom < p.bottom_min {
        cost += 100.0;
    }
    cost
}

/// Eigen's `sum()` of a float vector as the reference's build (arm64, NEON)
/// adds it (Redux.h, `LinearVectorizedTraversal`): two running sums of
/// four lanes, eight at a time, then four, the lanes added as
/// `(a0 + a1) + (a2 + a3)` (`predux`), then what is left one by one.
fn eigen_sum(values: &[f32]) -> f32 {
    let n = values.len();
    let size = n / 4 * 4;
    let size2 = n / 8 * 8;
    if size == 0 {
        return values
            .iter()
            .skip(1)
            .fold(values.first().copied().unwrap_or(0.0), |sum, v| sum + v);
    }
    let lanes = |at: usize| -> [f32; 4] { std::array::from_fn(|i| values[at + i]) };
    let add = |a: [f32; 4], b: [f32; 4]| -> [f32; 4] { std::array::from_fn(|i| a[i] + b[i]) };
    let mut p0 = lanes(0);
    if size > 4 {
        let mut p1 = lanes(4);
        let mut at = 8;
        while at < size2 {
            p0 = add(p0, lanes(at));
            p1 = add(p1, lanes(at + 4));
            at += 8;
        }
        p0 = add(p0, p1);
        if size > size2 {
            p0 = add(p0, lanes(size2));
        }
    }
    let mut sum = (p0[0] + p0[1]) + (p0[2] + p0[3]);
    for value in &values[size..] {
        sum += value;
    }
    sum
}

/// Which way up `mesh` prints best, `hull` being its convex hull
/// (Orient.cpp `AutoOrienter::process` and `_orient`). None for a mesh
/// with no faces.
pub fn orient(mesh: Faces<'_>, hull: Faces<'_>, params: &Params) -> Option<Orientation> {
    if mesh.triangles.is_empty() {
        return None;
    }
    let prepared = Prepared::new(mesh, hull);
    let tried: Vec<Tried> = prepared
        .candidates()
        .par_iter()
        .map(|down| {
            let up = down.map(|v| -v);
            let costs = prepared.cost(up, params);
            Tried { up, costs }
        })
        .collect();

    // Least cost first; equals keep the order tried.
    let mut order: Vec<usize> = (0..tried.len()).collect();
    order.sort_by(|a, b| {
        tried[*a]
            .costs
            .unprintability
            .total_cmp(&tried[*b].costs.unprintability)
    });
    // "To avoid flipping": among those tied with the best, the direction
    // that leaves the mesh as it stands.
    let n1 = [0.0f32, 0.0, 1.0];
    let first = &tried[order[0]];
    let mut best = first.up;
    for at in &order[1..] {
        let other = &tried[*at];
        let tied =
            ((other.costs.unprintability - first.costs.unprintability) as f64).abs() < EPSILON;
        if !(tied && ((dot(first.up, n1) - 1.0) as f64).abs() > EPSILON) {
            break;
        }
        if (((dot(other.up, n1) - 1.0) as f64).abs()) < EPSILON * EPSILON {
            best = n1;
            break;
        }
    }
    let up = best.map(f64::from);
    Some(Orientation {
        up,
        rotation: rotation_from_two_vectors(up, [0.0, 0.0, 1.0]),
        tried,
    })
}

/// Geometry.cpp `rotation_from_two_vectors`: Eigen's
/// `Quaternion::setFromTwoVectors(from, to).toRotationMatrix()`, with the
/// axis Eigen takes for opposite directions from its singular value
/// decomposition worked out as it works it out.
pub fn rotation_from_two_vectors(from: [f64; 3], to: [f64; 3]) -> Mat3 {
    let unit = |a: [f64; 3]| {
        let n2 = a[0] * a[0] + a[1] * a[1] + a[2] * a[2];
        if n2 > 0.0 {
            let n = n2.sqrt();
            a.map(|v| v / n)
        } else {
            a
        }
    };
    let (v0, v1) = (unit(from), unit(to));
    let c = v1[0] * v0[0] + v1[1] * v0[1] + v1[2] * v0[2];
    let (w, vec) = if c < -1.0 + 1e-12 {
        let c = c.max(-1.0);
        let axis = opposite_axis(v0, v1);
        let w2 = (1.0 + c) * 0.5;
        let s = (1.0 - w2).sqrt();
        (w2.sqrt(), axis.map(|v| v * s))
    } else {
        let axis = [
            v0[1] * v1[2] - v0[2] * v1[1],
            v0[2] * v1[0] - v0[0] * v1[2],
            v0[0] * v1[1] - v0[1] * v1[0],
        ];
        let s = ((1.0 + c) * 2.0).sqrt();
        let invs = 1.0 / s;
        (s * 0.5, axis.map(|v| v * invs))
    };
    let [x, y, z] = vec;
    let (tx, ty, tz) = (2.0 * x, 2.0 * y, 2.0 * z);
    let (twx, twy, twz) = (tx * w, ty * w, tz * w);
    let (txx, txy, txz) = (tx * x, ty * x, tz * x);
    let (tyy, tyz, tzz) = (ty * y, tz * y, tz * z);
    [
        [1.0 - (tyy + tzz), txy - twz, txz + twy],
        [txy + twz, 1.0 - (txx + tzz), tyz - twx],
        [txz - twy, tyz + twx, 1.0 - (txx + tyy)],
    ]
}

/// The axis Eigen turns about when `v0` and `v1` are opposite: the last
/// column of V in `JacobiSVD` of the 2 × 3 matrix of the two, which for a
/// matrix wider than tall is the last column of Q in the column-pivoting
/// Householder QR of its transpose (Q = H0 H1, so the column is H0 H1 e2).
fn opposite_axis(v0: [f64; 3], v1: [f64; 3]) -> [f64; 3] {
    let norm = |a: [f64; 3]| (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    // The column of the greater norm first; on a tie, the first.
    let (a, b) = if norm(v1) > norm(v0) {
        (v1, v0)
    } else {
        (v0, v1)
    };
    // Eigen's `makeHouseholder` of a vector: (tau, essential part).
    let householder = |c0: f64, tail: &[f64]| -> (f64, Vec<f64>) {
        let tail_sq: f64 = tail.iter().map(|v| v * v).sum();
        if tail_sq <= f64::MIN_POSITIVE {
            (0.0, vec![0.0; tail.len()])
        } else {
            let mut beta = (c0 * c0 + tail_sq).sqrt();
            if c0 >= 0.0 {
                beta = -beta;
            }
            (
                (beta - c0) / beta,
                tail.iter().map(|v| v / (c0 - beta)).collect(),
            )
        }
    };
    let (tau0, e0) = householder(a[0], &a[1..]);
    let h0 = |x: [f64; 3]| -> [f64; 3] {
        let v = [1.0, e0[0], e0[1]];
        let d = v[0] * x[0] + v[1] * x[1] + v[2] * x[2];
        std::array::from_fn(|i| x[i] - tau0 * v[i] * d)
    };
    let b = h0(b);
    let (tau1, e1) = householder(b[1], &b[2..]);
    // H1 e2, H1 acting on rows 1 and 2 with v = (1, e1).
    let y = [0.0, -tau1 * e1[0], 1.0 - tau1 * e1[0] * e1[0]];
    h0(y)
}

#[cfg(test)]
mod tests;
