//! What a stroke of paint takes in: a ball or a circle seen from the
//! camera, the two drawn out along the stroke, or a band of heights
//! (TriangleSelector.hpp `Cursor`, `Sphere`, `Circle`, `Capsule3D`,
//! `Capsule2D`, `HeightRange`; TriangleSelector.cpp, where their tests are).

use super::{Affine, V3, add, cross, dot, length, normalize, scale, sub};

/// A plane that hides what is in front of it: a point `p` (in the mesh's
/// coordinates) is cut away when `normal · p - offset > 0`
/// (TriangleSelector.hpp `ClippingPlane`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub normal: V3,
    pub offset: f32,
}

impl Clip {
    pub fn clipped(&self, point: V3) -> bool {
        dot(self.normal, point) - self.offset > 0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    /// A ball round `centre`.
    Sphere,
    /// A disc round `centre` square to the line of sight: everything that
    /// is under it as the camera sees it.
    Circle,
    /// A ball drawn from `centre` to `second`.
    Capsule3D,
    /// A disc drawn from `centre` to `second`.
    Capsule2D,
    /// Everything between `z` and `z + height` in the world's coordinates
    /// ("Height range").
    HeightRange { z: f32, height: f32 },
}

/// A cursor, in the mesh's coordinates while the mesh is scaled alike
/// along every axis, else in the world's.
#[derive(Clone, Debug)]
pub struct Cursor {
    pub(crate) shape: Shape,
    pub(crate) centre: V3,
    pub(crate) second: V3,
    pub(crate) trafo: Affine,
    pub(crate) uniform: bool,
    /// The inverse transpose of the trafo's linear part, for normals.
    pub(crate) trafo_normal: [V3; 3],
    pub(crate) radius: f32,
    pub(crate) radius_sqr: f32,
    pub(crate) dir: V3,
    pub(crate) clip: Option<Clip>,
}

/// TriangleSelector.cpp `test_line_inside_sphere`.
fn line_in_sphere(a: V3, b: V3, p: V3, radius: f32) -> bool {
    let r2 = radius * radius;
    let n = sub(b, a);
    let m = sub(a, p);
    let mm = dot(m, m);
    if mm <= r2 || dot(sub(b, p), sub(b, p)) <= r2 {
        return true;
    }
    let nn = dot(n, n);
    let mn = dot(m, n);
    let discr = mn * mn - nn * (mm - r2);
    if discr < 0.0 {
        return false;
    }
    let root = discr.sqrt();
    let t1 = (-mn - root) / nn;
    if (0.0..=1.0).contains(&t1) {
        return true;
    }
    let t2 = (-mn + root) / nn;
    (0.0..=1.0).contains(&t2) && root > 0.0
}

/// TriangleSelector.cpp `test_line_inside_cylinder`.
fn line_in_cylinder(a: V3, b: V3, p: V3, q: V3, radius: f32) -> bool {
    let d = sub(q, p);
    let inside = |pt: V3| {
        let first = sub(p, pt);
        let second = sub(q, pt);
        dot(first, d) <= 0.0
            && dot(second, d) >= 0.0
            && length(cross(first, d)) / length(d) <= radius
    };
    if inside(a) || inside(b) {
        return true;
    }
    let m = sub(a, p);
    let n = sub(b, a);
    let md = dot(m, d);
    let nd = dot(n, d);
    let dd = dot(d, d);
    let nn = dot(n, n);
    let mn = dot(m, n);
    let mm = dot(m, m);
    let eq_a = dd * nn - nd * nd;
    let eq_b = dd * mn - nd * md;
    let eq_c = dd * (mm - radius * radius) - md * md;
    let discr = eq_b * eq_b - eq_a * eq_c;
    if discr < 0.0 {
        return false;
    }
    let root = discr.sqrt();
    let t1 = (-eq_b - root) / eq_a;
    if (0.0..=1.0).contains(&t1) && (0.0..=dd).contains(&(md + t1 * nd)) {
        return true;
    }
    let t2 = (-eq_b + root) / eq_a;
    (0.0..=1.0).contains(&t2) && (0.0..=dd).contains(&(md + t2 * nd))
}

/// TriangleSelector.cpp `test_line_inside_capsule`.
fn line_in_capsule(a: V3, b: V3, p: V3, q: V3, radius: f32) -> bool {
    line_in_sphere(a, b, p, radius)
        || line_in_sphere(a, b, q, radius)
        || line_in_cylinder(a, b, p, q, radius)
}

/// TriangleSelector.cpp `is_circle_pointer_inside_triangle`: whether the
/// line of sight through `centre` passes through the triangle.
fn line_of_sight_through(p: [V3; 3], centre: V3, dir: V3) -> bool {
    let q1 = add(centre, dir);
    let q2 = sub(centre, dir);
    let sign = |a: V3, b: V3, c: V3, d: V3| dot(cross(sub(b, a), sub(c, a)), sub(d, a)) > 0.0;
    if sign(q1, p[0], p[1], p[2]) == sign(q2, p[0], p[1], p[2]) {
        return false;
    }
    let pos = sign(q1, q2, p[0], p[1]);
    sign(q1, q2, p[1], p[2]) == pos && sign(q1, q2, p[2], p[0]) == pos
}

/// TriangleSelector.cpp `line_plane_intersection`.
fn line_plane(a: V3, b: V3, origin: V3, normal: V3) -> Option<V3> {
    let dir = sub(b, a);
    let den = dot(normal, dir);
    if den == 0.0 {
        return None;
    }
    let t = (dot(normal, origin) - dot(normal, a)) / den;
    (0.0..=1.0).contains(&t).then(|| add(a, scale(dir, t)))
}

impl Cursor {
    /// A cursor of `shape` at `centre` (and `second` for the drawn
    /// shapes), seen from `source` (the camera, in the mesh's coordinates),
    /// `radius` millimetres in the world, for a mesh that `trafo` places in
    /// the world (`Cursor::Cursor`, `SinglePointCursor`,
    /// `DoublePointCursor`). A height range is placed by its own `z` and
    /// `height`, its radius one millimetre.
    pub fn new(
        shape: Shape,
        centre: V3,
        second: V3,
        source: V3,
        radius: f32,
        trafo: Affine,
        clip: Option<Clip>,
    ) -> Cursor {
        let factors = trafo.scaling_factors();
        let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;
        let uniform = approx(factors[0], factors[1]) && approx(factors[1], factors[2]);
        let (mut source, mut centre, mut second) = (source, centre, second);
        let (radius_mesh, trafo_normal) = if uniform {
            (
                (f64::from(radius) / factors[0]) as f32,
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            )
        } else {
            source = trafo.apply(source);
            centre = trafo.apply(centre);
            second = trafo.apply(second);
            (radius, trafo.normal_matrix())
        };
        let radius_sqr = if uniform {
            let r = f64::from(radius) / factors[0];
            (r * r) as f32
        } else {
            radius * radius
        };
        let dir = normalize(sub(centre, source));
        Cursor {
            shape,
            centre,
            second,
            trafo,
            uniform,
            trafo_normal,
            radius: radius_mesh,
            radius_sqr,
            dir,
            clip,
        }
    }

    fn placed(&self, point: V3) -> V3 {
        if self.uniform {
            point
        } else {
            self.trafo.apply(point)
        }
    }

    fn not_clipped(&self, point: V3) -> bool {
        self.clip.is_none_or(|clip| !clip.clipped(point))
    }

    /// Whether a point of the mesh is inside (`is_mesh_point_inside`).
    pub(crate) fn point_inside(&self, point: V3) -> bool {
        match self.shape {
            Shape::Sphere => {
                let p = self.placed(point);
                let d = sub(self.centre, p);
                dot(d, d) < self.radius_sqr && self.not_clipped(point)
            }
            Shape::Circle => {
                let p = self.placed(point);
                let d = sub(self.centre, p);
                let across = sub(d, scale(self.dir, dot(d, self.dir)));
                dot(across, across) < self.radius_sqr && self.not_clipped(point)
            }
            Shape::Capsule3D => {
                let p = self.placed(point);
                let first = sub(self.centre, p);
                let second = sub(self.second, p);
                if dot(first, first) < self.radius_sqr || dot(second, second) < self.radius_sqr {
                    return self.not_clipped(point);
                }
                let along = sub(self.second, self.centre);
                if dot(first, along) <= 0.0
                    && dot(second, along) >= 0.0
                    && length(cross(first, along)) / length(along) <= self.radius
                {
                    return self.not_clipped(point);
                }
                false
            }
            Shape::Capsule2D => {
                let p = self.placed(point);
                let first = sub(self.centre, p);
                let first_across = sub(first, scale(self.dir, dot(first, self.dir)));
                if dot(first_across, first_across) < self.radius_sqr {
                    return self.not_clipped(point);
                }
                let second = sub(self.second, p);
                let second_across = sub(second, scale(self.dir, dot(second, self.dir)));
                if dot(second_across, second_across) < self.radius_sqr {
                    return self.not_clipped(point);
                }
                let along = sub(self.second, self.centre);
                let along_across = sub(along, scale(self.dir, dot(along, self.dir)));
                if dot(first_across, along_across) <= 0.0 && dot(second_across, along_across) >= 0.0
                {
                    let da = cross(along, self.dir);
                    let to_a = scale(normalize(da), self.radius);
                    let a = sub(self.centre, to_a);
                    let d = add(self.centre, to_a);
                    if dot(sub(a, p), da) <= 0.0 && dot(sub(d, p), da) >= 0.0 {
                        return self.not_clipped(point);
                    }
                }
                false
            }
            Shape::HeightRange { z, height } => {
                // The reference allows 0.02 mm either side.
                let tolerance = 0.02;
                let p = self.trafo.apply(point);
                p[2] > z - tolerance && p[2] < z + height + tolerance
            }
        }
    }

    /// Whether the line of sight through the cursor's middle (or either
    /// middle) passes through a triangle (`is_pointer_in_triangle`).
    pub(crate) fn pointer_in(&self, corners: [V3; 3]) -> bool {
        let p = corners.map(|corner| self.placed(corner));
        match self.shape {
            Shape::HeightRange { .. } => false,
            Shape::Sphere | Shape::Circle => line_of_sight_through(p, self.centre, self.dir),
            Shape::Capsule3D | Shape::Capsule2D => {
                line_of_sight_through(p, self.centre, self.dir)
                    || line_of_sight_through(p, self.second, self.dir)
            }
        }
    }

    /// Whether any side of a triangle passes through (`is_edge_inside_cursor`).
    pub(crate) fn edge_inside(&self, corners: [V3; 3]) -> bool {
        if let Shape::HeightRange { z, height } = self.shape {
            let eps = 1e-4;
            let (top, bottom) = (z + height + eps, z - eps);
            let p = corners.map(|corner| self.trafo.apply(corner));
            return !(p.iter().all(|p| p[2] < bottom) || p.iter().all(|p| p[2] > top));
        }
        let pts = corners.map(|corner| self.placed(corner));
        let sides = [(pts[0], pts[1]), (pts[1], pts[2]), (pts[2], pts[0])];
        match self.shape {
            Shape::Sphere => sides
                .iter()
                .any(|&(a, b)| line_in_sphere(a, b, self.centre, self.radius)),
            Shape::Capsule3D => sides
                .iter()
                .any(|&(a, b)| line_in_capsule(a, b, self.centre, self.second, self.radius)),
            Shape::Circle => sides.iter().any(|&(a, b)| {
                let s = normalize(sub(b, a));
                let t = dot(sub(self.centre, a), s);
                let v = sub(add(a, scale(s, t)), self.centre);
                let dist_sqr = dot(v, v) - dot(v, self.dir).powi(2);
                dist_sqr < self.radius_sqr && t >= 0.0 && t <= length(sub(b, a))
            }),
            Shape::Capsule2D => {
                let along = sub(self.second, self.centre);
                let da = cross(along, self.dir);
                let to_a = scale(normalize(da), self.radius);
                let ra = sub(self.centre, to_a);
                let rd = add(self.centre, to_a);
                let in_rectangle = |a: V3, b: V3, origin: V3, normal: V3| {
                    line_plane(a, b, origin, normal).is_some_and(|hit| {
                        dot(self.centre, along) <= dot(hit, along)
                            && dot(hit, along) <= dot(self.second, along)
                    })
                };
                sides.iter().any(|&(a, b)| {
                    let edge = sub(b, a);
                    let n = normalize(edge);
                    let reach = length(edge);
                    let t1 = dot(sub(self.centre, a), n);
                    let t2 = dot(sub(self.second, a), n);
                    let v1 = sub(add(a, scale(n, t1)), self.centre);
                    let v2 = sub(add(a, scale(n, t2)), self.second);
                    (dot(v1, v1) - dot(v1, self.dir).powi(2) < self.radius_sqr
                        && t1 >= 0.0
                        && t1 <= reach)
                        || (dot(v2, v2) - dot(v2, self.dir).powi(2) < self.radius_sqr
                            && t2 >= 0.0
                            && t2 <= reach)
                        || in_rectangle(a, b, ra, sub(rd, ra))
                        || in_rectangle(a, b, rd, sub(ra, rd))
                })
            }
            Shape::HeightRange { .. } => unreachable!(),
        }
    }

    /// Whether a triangle with this normal may be seen: the circles paint
    /// only what faces the camera (`is_facet_visible`).
    pub(crate) fn facet_visible(&self, normal: V3) -> bool {
        match self.shape {
            Shape::Circle | Shape::Capsule2D => {
                let n = if self.uniform {
                    normal
                } else {
                    let m = &self.trafo_normal;
                    [dot(m[0], normal), dot(m[1], normal), dot(m[2], normal)]
                };
                dot(n, self.dir) < 0.0
            }
            _ => true,
        }
    }
}
