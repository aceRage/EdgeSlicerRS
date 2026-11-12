//! "Cut" with a curved surface (the reference fork's CurvedCut.cpp): a
//! height field over the cut plane, given by a grid of control points and
//! blended between them (Catmull-Rom; straight with two points a side).
//! The object is cut with the closed solid under the surface (a slab whose
//! top is the surface), by a mesh boolean: the lower half is the object's
//! intersection with it, the upper half the object less it (`cut_with_solid`).
//!
//! Coordinates here are the plane's own: x and y in it, z along its
//! normal, the upper half where z is greater.

use edge_mesh::boolean::{self, BooleanError, Cancel, Op, Options, Solid};
use edge_mesh::{Mesh, Transform};

/// The fewest and the most control points a side (`MinResolution`,
/// `MaxResolution`), and how finely the cut samples the surface
/// (`CutSamples`).
pub const SHEET_MIN: usize = 2;
pub const SHEET_MAX: usize = 15;
pub const CUT_SAMPLES: usize = 128;

/// The surface (`CurvedCutSheet`): `nx` × `ny` control points spread
/// evenly over `[-half_u, half_u]` × `[-half_v, half_v]`, each at height
/// `z[j * nx + i]` over the plane. Beyond its edges it goes on level.
#[derive(Clone, Debug, PartialEq)]
pub struct Sheet {
    pub nx: usize,
    pub ny: usize,
    pub half_u: f64,
    pub half_v: f64,
    pub z: Vec<f64>,
}

/// Catmull-Rom between `p1` and `p2`.
fn catmull_rom(p0: f64, p1: f64, p2: f64, p3: f64, t: f64) -> f64 {
    let (t2, t3) = (t * t, t * t * t);
    0.5 * ((2.0 * p1)
        + (-p0 + p2) * t
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2
        + (-p0 + 3.0 * p1 - 3.0 * p2 + p3) * t3)
}

impl Sheet {
    /// A flat sheet.
    pub fn flat(nx: usize, ny: usize, half_u: f64, half_v: f64) -> Sheet {
        let (nx, ny) = (
            nx.clamp(SHEET_MIN, SHEET_MAX),
            ny.clamp(SHEET_MIN, SHEET_MAX),
        );
        Sheet {
            nx,
            ny,
            half_u: half_u.max(1e-6),
            half_v: half_v.max(1e-6),
            z: vec![0.0; nx * ny],
        }
    }

    /// Whether it is a sheet: its grid within bounds, its values finite.
    pub fn sound(&self) -> bool {
        (SHEET_MIN..=SHEET_MAX).contains(&self.nx)
            && (SHEET_MIN..=SHEET_MAX).contains(&self.ny)
            && self.z.len() == self.nx * self.ny
            && self.z.iter().all(|z| z.is_finite())
            && self.half_u > 0.0
            && self.half_v > 0.0
    }

    pub fn is_flat(&self) -> bool {
        self.z.iter().all(|z| *z == 0.0)
    }

    /// The furthest any control point stands off the plane.
    pub fn max_displacement(&self) -> f64 {
        self.z.iter().fold(0.0, |most: f64, z| most.max(z.abs()))
    }

    fn at(&self, i: isize, j: isize) -> f64 {
        let i = i.clamp(0, self.nx as isize - 1) as usize;
        let j = j.clamp(0, self.ny as isize - 1) as usize;
        self.z[j * self.nx + i]
    }

    /// The height at `(u, v)` in `[0, 1]²` (`evaluate`).
    pub fn evaluate(&self, u: f64, v: f64) -> f64 {
        if self.nx < 2 || self.ny < 2 || self.is_flat() {
            return 0.0;
        }
        let fu = u.clamp(0.0, 1.0) * (self.nx - 1) as f64;
        let fv = v.clamp(0.0, 1.0) * (self.ny - 1) as f64;
        let iu = (fu.floor() as isize).clamp(0, self.nx as isize - 2);
        let iv = (fv.floor() as isize).clamp(0, self.ny as isize - 2);
        let (tu, tv) = (fu - iu as f64, fv - iv as f64);
        let blend = |p: [f64; 4], t: f64, count: usize| {
            if count == 2 {
                p[1] + (p[2] - p[1]) * t
            } else {
                catmull_rom(p[0], p[1], p[2], p[3], t)
            }
        };
        let column: [f64; 4] = std::array::from_fn(|k| {
            let j = iv - 1 + k as isize;
            blend(
                [
                    self.at(iu - 1, j),
                    self.at(iu, j),
                    self.at(iu + 1, j),
                    self.at(iu + 2, j),
                ],
                tu,
                self.nx,
            )
        });
        blend(column, tv, self.ny)
    }

    /// The height over the plane's point `(x, y)` (`evaluate_local`).
    pub fn height(&self, x: f64, y: f64) -> f64 {
        self.evaluate(0.5 * (x / self.half_u + 1.0), 0.5 * (y / self.half_v + 1.0))
    }

    /// The sheet turned over with the plane flipped about its x (y and z
    /// both turned: `flip_about_u`).
    pub fn flipped(&self) -> Sheet {
        let mut out = self.clone();
        for j in 0..self.ny {
            for i in 0..self.nx {
                out.z[j * self.nx + i] = -self.z[(self.ny - 1 - j) * self.nx + i];
            }
        }
        out
    }

    /// The same surface mirrored across the plane's y (the grid's rows in
    /// reverse, the heights kept: `cut_recipe_mirrored`).
    pub fn mirrored(&self) -> Sheet {
        let mut out = self.clone();
        for j in 0..self.ny {
            for i in 0..self.nx {
                out.z[j * self.nx + i] = self.z[(self.ny - 1 - j) * self.nx + i];
            }
        }
        out
    }
}

/// How far a cut's faces stand off the surface (`curved_cut_thickness_faces`
/// as [`crate::CutOptions::faces`] gives them).
///
/// The closed solid under the surface moved up by `offset`, reaching
/// `extent` each way in the plane and down below everything `low`
/// reaches (`curved_cut_lower_slab`), sampled `samples` a side.
pub fn lower_slab(
    sheet: &Sheet,
    low: f64,
    diagonal: f64,
    samples: usize,
    extent: [f64; 2],
    offset: f64,
) -> Mesh {
    let n = samples.max(2);
    let slack = (0.1 * diagonal).max(1.0);
    let floor = low.min(-sheet.max_displacement() + offset.min(0.0)) - slack;
    let half_u = extent[0].max(sheet.half_u);
    let half_v = extent[1].max(sheet.half_v);
    let mut mesh = Mesh::default();
    let at = |k: usize, half: f64| (2.0 * (k as f64 / (n - 1) as f64) - 1.0) * half;
    for j in 0..n {
        for i in 0..n {
            let (x, y) = (at(i, half_u), at(j, half_v));
            mesh.vertices
                .push([x as f32, y as f32, (sheet.height(x, y) + offset) as f32]);
        }
    }
    for j in 0..n {
        for i in 0..n {
            mesh.vertices
                .push([at(i, half_u) as f32, at(j, half_v) as f32, floor as f32]);
        }
    }
    let n32 = n as u32;
    let top = |i: u32, j: u32| j * n32 + i;
    let bottom = |i: u32, j: u32| n32 * n32 + j * n32 + i;
    for j in 0..n32 - 1 {
        for i in 0..n32 - 1 {
            mesh.triangles
                .push([top(i, j), top(i + 1, j), top(i + 1, j + 1)]);
            mesh.triangles
                .push([top(i, j), top(i + 1, j + 1), top(i, j + 1)]);
            mesh.triangles
                .push([bottom(i, j), bottom(i + 1, j + 1), bottom(i + 1, j)]);
            mesh.triangles
                .push([bottom(i, j), bottom(i, j + 1), bottom(i + 1, j + 1)]);
        }
    }
    let last = n32 - 1;
    for i in 0..n32 - 1 {
        mesh.triangles
            .push([top(i, 0), bottom(i, 0), bottom(i + 1, 0)]);
        mesh.triangles
            .push([top(i, 0), bottom(i + 1, 0), top(i + 1, 0)]);
        mesh.triangles
            .push([top(i, last), top(i + 1, last), bottom(i + 1, last)]);
        mesh.triangles
            .push([top(i, last), bottom(i + 1, last), bottom(i, last)]);
    }
    for j in 0..n32 - 1 {
        mesh.triangles
            .push([top(0, j), top(0, j + 1), bottom(0, j + 1)]);
        mesh.triangles
            .push([top(0, j), bottom(0, j + 1), bottom(0, j)]);
        mesh.triangles
            .push([top(last, j), bottom(last, j), bottom(last, j + 1)]);
        mesh.triangles
            .push([top(last, j), bottom(last, j + 1), top(last, j + 1)]);
    }
    mesh
}

/// Why a curved cut gave no halves.
#[derive(Clone, Debug, PartialEq)]
pub enum CurvedError {
    Boolean(BooleanError),
    Input(boolean::InputError),
    Cancelled,
}

impl std::fmt::Display for CurvedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Boolean(error) => write!(f, "{error}"),
            Self::Input(error) => write!(f, "the mesh {error}"),
            Self::Cancelled => f.write_str("the cut was stopped"),
        }
    }
}

impl std::error::Error for CurvedError {}

impl From<BooleanError> for CurvedError {
    fn from(error: BooleanError) -> Self {
        Self::Boolean(error)
    }
}

impl From<boolean::InputError> for CurvedError {
    fn from(error: boolean::InputError) -> Self {
        Self::Input(error)
    }
}

/// The two halves of `object` (already prepared, in the plane's frame or
/// any other, with the slabs in the same) by the slab `lower` and, with a
/// cut thickness, `upper` (`cut_with_solid`): the lower half is what of
/// the object is in `lower`, the upper what is not in `upper`. Without a
/// thickness, a half the boolean gives nothing for is taken as the object
/// less the other, as the reference recovers it. An empty half is an empty
/// mesh.
pub fn cut_with_solid(
    object: &Solid,
    lower: &Mesh,
    upper: Option<&Mesh>,
    cancel: Option<&Cancel>,
) -> Result<(Mesh, Mesh), CurvedError> {
    let options = Options {
        cancel: cancel.cloned(),
        ..Options::default()
    };
    let stopped = || cancel.is_some_and(Cancel::is_cancelled);
    let low = Solid::new(lower)?;
    let high = match upper {
        Some(mesh) => Some(Solid::new(mesh)?),
        None => None,
    };
    let run = |op: Op, with: &Solid| -> Result<Solid, CurvedError> {
        object.boolean_with(with, op, &options).map_err(|error| {
            if stopped() {
                CurvedError::Cancelled
            } else {
                CurvedError::Boolean(error)
            }
        })
    };
    let below = run(Op::Intersection, &low)?;
    let above = run(Op::Difference, high.as_ref().unwrap_or(&low))?;
    let (mut below_mesh, mut above_mesh) = (below.mesh()?, above.mesh()?);
    if upper.is_none() {
        if above_mesh.triangles.is_empty() && !below_mesh.triangles.is_empty() {
            above_mesh = run(Op::Difference, &below)?.mesh()?;
        } else if below_mesh.triangles.is_empty() && !above_mesh.triangles.is_empty() {
            below_mesh = run(Op::Difference, &above)?.mesh()?;
        }
    }
    if stopped() {
        return Err(CurvedError::Cancelled);
    }
    Ok((tidy(above_mesh), tidy(below_mesh)))
}

/// Corners the boolean left a hair apart made one, so that a half placed
/// on the bed cannot fold a triangle to no area in single precision
/// ([`edge_mesh::tidy::weld_close`], within the cut's own 1e-4 mm); the
/// mesh as it was when that would open it.
pub fn tidy(mesh: Mesh) -> Mesh {
    if mesh.triangles.is_empty() {
        return mesh;
    }
    edge_mesh::tidy::weld_close(&mesh, 1e-4).unwrap_or(mesh)
}

/// The slabs a cut by `sheet` takes, in the frame `plane` takes the
/// plane's own coordinates to, for a mesh whose points `points` (in that
/// frame) are: one at the surface, or one at each face with a thickness
/// (`curved_cut_split`). `faces` are the lower and upper faces' offsets.
pub fn slabs(
    sheet: &Sheet,
    plane: &Transform,
    points: &[[f32; 3]],
    faces: [f64; 2],
) -> (Mesh, Option<Mesh>) {
    let into = plane.inverse().unwrap_or(Transform::IDENTITY);
    let (mut low, mut high) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for point in points {
        let p = into.apply(point.map(f64::from));
        for axis in 0..3 {
            low[axis] = low[axis].min(p[axis]);
            high[axis] = high[axis].max(p[axis]);
        }
    }
    let (low, high) = if points.is_empty() {
        ([0.0; 3], [0.0; 3])
    } else {
        (low, high)
    };
    let diagonal = (0..3)
        .map(|a| (high[a] - low[a]).powi(2))
        .sum::<f64>()
        .sqrt();
    let reach = |a: usize| 1.05 * low[a].abs().max(high[a].abs()) + 1.0;
    let extent = [reach(0), reach(1)];
    let placed = |offset: f64| {
        let slab = lower_slab(sheet, low[2], diagonal, CUT_SAMPLES, extent, offset);
        let mut out = Mesh::default();
        out.append(&slab, plane);
        out
    };
    if faces[0] == faces[1] {
        (placed(faces[0]), None)
    } else {
        (placed(faces[0]), Some(placed(faces[1])))
    }
}

/// Whether a side of the surface has none of the mesh over it (or under
/// it): `[upper empty, lower empty]` (`curved_cut_empty_sides`), the mesh's
/// points in the plane's own frame.
pub fn empty_sides(points: &[[f64; 3]], sheet: &Sheet, faces: [f64; 2]) -> [bool; 2] {
    let (mut upper, mut lower) = (true, true);
    for p in points {
        let d = p[2] - sheet.height(p[0], p[1]);
        if d > faces[1] {
            upper = false;
        } else if d < faces[0] {
            lower = false;
        }
        if !upper && !lower {
            break;
        }
    }
    [upper, lower]
}

/// How far along z a point must go to meet the mesh's surface, the
/// nearest way (`curved_cut_snap_distance`): over every triangle the
/// vertical line through `point` crosses. None when it crosses none.
pub fn snap_distance(mesh: &Mesh, point: [f64; 3]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for triangle in &mesh.triangles {
        let [a, b, c] = triangle.map(|v| mesh.vertices[v as usize].map(f64::from));
        let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
        if d.abs() < 1e-12 {
            continue;
        }
        let l0 = ((b[1] - c[1]) * (point[0] - c[0]) + (c[0] - b[0]) * (point[1] - c[1])) / d;
        let l1 = ((c[1] - a[1]) * (point[0] - c[0]) + (a[0] - c[0]) * (point[1] - c[1])) / d;
        let l2 = 1.0 - l0 - l1;
        if l0 < -1e-9 || l1 < -1e-9 || l2 < -1e-9 {
            continue;
        }
        let along = l0 * a[2] + l1 * b[2] + l2 * c[2] - point[2];
        if best.is_none_or(|held| along.abs() < held.abs()) {
            best = Some(along);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::primitives::cuboid;

    #[test]
    fn a_sheet_is_level_at_its_points_and_straight_with_two() {
        let mut sheet = Sheet::flat(3, 2, 10.0, 5.0);
        assert!(sheet.is_flat());
        sheet.z[1] = 2.0;
        // At its control points it is what they say.
        assert!((sheet.height(0.0, -5.0) - 2.0).abs() < 1e-12);
        assert!(sheet.height(-10.0, -5.0).abs() < 1e-12);
        // Two a side down: straight between them.
        assert!((sheet.height(0.0, 0.0) - 1.0).abs() < 1e-12);
        // Beyond the edge it goes on level.
        assert!((sheet.height(0.0, -50.0) - 2.0).abs() < 1e-12);
        assert_eq!(sheet.flipped().flipped(), sheet);
        assert_eq!(sheet.mirrored().mirrored(), sheet);
    }

    #[test]
    fn a_block_cut_by_a_bent_sheet_comes_apart_closed_and_whole() {
        let block = Mesh::from_triangle_soup(cuboid([-10.0, -10.0, -10.0], [10.0, 10.0, 10.0]));
        let solid = Solid::new(&block).unwrap();
        let mut sheet = Sheet::flat(5, 5, 12.0, 12.0);
        sheet.z[12] = 4.0;
        for (faces, gap) in [([0.0, 0.0], false), ([-0.5, 0.5], true)] {
            let (low, high) = slabs(&sheet, &Transform::IDENTITY, &block.vertices, faces);
            let (upper, lower) = cut_with_solid(&solid, &low, high.as_ref(), None).unwrap();
            for half in [&upper, &lower] {
                assert!(edge_mesh::cut::is_closed(half));
                assert!(half.signed_volume() > 0.0);
            }
            let sum = upper.signed_volume() + lower.signed_volume();
            if gap {
                assert!(sum < 8000.0 - 300.0 && sum > 8000.0 - 500.0, "{sum}");
            } else {
                assert!((sum - 8000.0).abs() < 1e-3, "{sum}");
                // The bump: the lower half is more than half.
                assert!(lower.signed_volume() > 4000.0 + 50.0);
            }
        }
        // A surface over the whole block: everything below, nothing above.
        let mut high = Sheet::flat(2, 2, 12.0, 12.0);
        high.z = vec![20.0; 4];
        let (low, _) = slabs(&high, &Transform::IDENTITY, &block.vertices, [0.0, 0.0]);
        let (upper, lower) = cut_with_solid(&solid, &low, None, None).unwrap();
        assert!(upper.triangles.is_empty());
        assert!((lower.signed_volume() - 8000.0).abs() < 1e-3);
        // Snapping: straight up from under the block's middle to its floor.
        assert!((snap_distance(&block, [0.5, 0.5, -15.0]).unwrap() - 5.0).abs() < 1e-9);
        assert_eq!(snap_distance(&block, [50.0, 0.0, 0.0]), None);
        let points: Vec<[f64; 3]> = block.vertices.iter().map(|v| v.map(f64::from)).collect();
        assert_eq!(empty_sides(&points, &high, [0.0, 0.0]), [true, false]);
    }
}
