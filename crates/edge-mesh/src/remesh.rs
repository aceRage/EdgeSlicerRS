//! "Repair/Remesh": a part rebuilt as the surface of its signed distance
//! field, the reference's cross-platform repair (GUI_ObjectList.cpp
//! `repair_by_remesh`), with its options (libslic3r/MeshRemesh.cpp
//! `remesh_with_options`): the base kept flat by remeshing only what is
//! above a slab of it (`remesh_keep_flat_bottom`) and the rebuilt corners
//! pulled back onto the part's sharp edges (`detect_sharp_edges`,
//! `snap_to_sharp_edges`).
//!
//! The field and its surface are where this differs from the reference,
//! which hands both to OpenVDB (OpenVDBUtils.cpp `remesh_by_voxels`:
//! `meshToVolume` with a band of three voxels each side, the shells joined
//! by `csgUnion`, `volumeToMesh` at 0 with no adaptivity, its quads cut in
//! two). OpenVDB is a library of its own; here [`voxel_remesh`] does the
//! same steps in its own way, so the result is the reference's in kind
//! (closed, wound outward, about two triangles to a voxel of surface,
//! detail under a voxel lost) and not to the triangle:
//!
//! - the mesh is cut into its shells and a shell of no volume (flat, or
//!   inside out) is left out, as `mesh_to_grid` does (`its_volume < EPSILON`);
//! - each grid point within three voxels of a shell gets its distance to
//!   the shell, negative inside, inside told by the parity of a ray along
//!   each of the three axes, two of three deciding (the reference's
//!   OpenVDB tells it by flood-filling the band's signs); the shells are
//!   joined by the least of their values, as `csgUnion` joins them;
//! - the surface is taken from the field as OpenVDB's `volumeToMesh` takes
//!   it at no adaptivity, a point in each grid cell the surface crosses (at
//!   the mean of the places it crosses the cell's edges) and a quad across
//!   each grid edge it crosses, made of the points of the four cells round
//!   that edge; a cell the surface crosses more than once gets a point for
//!   each patch of it (the corners inside joined only by the cell's
//!   edges), so the result is closed;
//! - where every shell is left out, the reference's `mesh_to_grid` falls
//!   back on the whole mesh without its voxel scale (one voxel a
//!   millimetre: OpenVDBUtils.cpp l.88-92, `TriangleMeshDataAdapter{mesh}`),
//!   an obvious slip; here the whole mesh is taken at the voxel size asked.

use std::collections::HashMap;

use rayon::prelude::*;

use crate::{Mesh, Transform, simplify::Cancelled};

/// MeshRemesh.hpp's clamps.
pub const VOXEL_MIN: f64 = 0.02;
pub const VOXEL_MAX: f64 = 5.0;
pub const ANGLE_MIN: f64 = 5.0;
pub const ANGLE_MAX: f64 = 175.0;
pub const BOTTOM_MARGIN_MAX: f64 = 50.0;
/// The bottom slab's height when none is given, in voxels.
pub const BOTTOM_MARGIN_VOXELS: f64 = 2.5;
/// A corner within this many voxels of the lowest is on the base.
pub const BASE_EPS_VOXELS: f64 = 0.05;
/// `meshToVolume`'s band each side, in voxels (`remesh_by_voxels`: 3, 3).
const BAND: f32 = 3.0;
/// libslic3r.h `EPSILON`: a shell of less volume is left out.
const EPSILON: f64 = 1e-4;

/// What the Remesh dialog collects (`RemeshOptions`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Voxel size in mm; 0 or less: [`auto_voxel_size`].
    pub voxel_size: f64,
    /// "Keep the bottom flat".
    pub keep_bottom_flat: bool,
    /// "Bottom slab height" in mm; 0 or less: [`BOTTOM_MARGIN_VOXELS`] voxels.
    pub bottom_margin: f64,
    /// "Preserve sharp edges".
    pub preserve_sharp_edges: bool,
    /// "Feature angle", degrees.
    pub sharp_feature_angle: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            voxel_size: 0.0,
            keep_bottom_flat: true,
            bottom_margin: 0.0,
            preserve_sharp_edges: true,
            sharp_feature_angle: 45.0,
        }
    }
}

/// What a remesh did (`RemeshReport`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub triangles_before: usize,
    pub triangles_after: usize,
    /// The voxel size used.
    pub voxel_size: f64,
    /// The flat bottom was asked for and kept.
    pub kept_bottom_flat: bool,
    /// The flat bottom was asked for and could not be kept: `note` says why.
    pub fell_back: bool,
    /// How many corners were pulled onto a sharp edge.
    pub sharp_snapped: usize,
    pub note: String,
}

/// `remesh_auto_voxel_size`: the box's diagonal over 300, 0.05 to 0.3 mm.
pub fn auto_voxel_size(mesh: &Mesh) -> f64 {
    match mesh.bounds() {
        Some((low, high)) if !mesh.triangles.is_empty() => {
            let d: f64 = (0..3)
                .map(|i| (f64::from(high[i]) - f64::from(low[i])).powi(2))
                .sum::<f64>()
                .sqrt();
            (d / 300.0).clamp(0.05, 0.3)
        }
        _ => 0.1,
    }
}

/// `remesh_with_options`: `mesh` remeshed in its own frame, `bed_dir` the
/// direction of the bed in that frame. None when it could not be done
/// (nothing left of it).
pub fn remesh_with_options(
    mesh: &Mesh,
    options: &Options,
    bed_dir: [f64; 3],
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<Option<(Mesh, Report)>, Cancelled> {
    let mut report = Report {
        triangles_before: mesh.triangles.len(),
        ..Report::default()
    };
    if mesh.triangles.is_empty() {
        return Ok(None);
    }
    let voxel = if options.voxel_size <= 0.0 {
        auto_voxel_size(mesh)
    } else {
        options.voxel_size
    }
    .clamp(VOXEL_MIN, VOXEL_MAX);
    report.voxel_size = voxel;
    let mut out = None;
    if options.keep_bottom_flat {
        match keep_flat_bottom(mesh, voxel, options.bottom_margin, bed_dir, cancel)? {
            Ok(made) => {
                report.kept_bottom_flat = true;
                out = Some(made);
            }
            Err(why) => {
                report.fell_back = true;
                report.note = why.to_owned();
            }
        }
    }
    let mut out = match out {
        Some(made) => made,
        None => match voxel_remesh(mesh, voxel, cancel)? {
            Some(made) => made,
            None => return Ok(None),
        },
    };
    if options.preserve_sharp_edges {
        let angle = options.sharp_feature_angle.clamp(ANGLE_MIN, ANGLE_MAX);
        let edges = detect_sharp_edges(mesh, angle);
        report.sharp_snapped = snap_to_sharp_edges(&mut out, &edges, voxel);
    }
    report.triangles_after = out.triangles.len();
    Ok(Some((out, report)))
}

/// `remesh_height`: how high a corner stands over the bed.
fn height(d: [f64; 3], v: [f32; 3]) -> f64 {
    -(d[0] * f64::from(v[0]) + d[1] * f64::from(v[1]) + d[2] * f64::from(v[2]))
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if n > 0.0 { v.map(|c| c / n) } else { v }
}

/// `remesh_bottom_area`: the area, seen from the bed, of the triangles
/// facing it whose corners are all within `eps` of the lowest; and how
/// many there are.
pub fn bottom_area(mesh: &Mesh, bed_dir: [f64; 3], eps: f64) -> (f64, usize) {
    if mesh.triangles.is_empty() {
        return (0.0, 0);
    }
    let d = unit(bed_dir);
    let z_min = mesh
        .vertices
        .iter()
        .map(|&v| height(d, v))
        .fold(f64::MAX, f64::min);
    let (mut area, mut count) = (0.0, 0);
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.vertices[i as usize]);
        if [a, b, c].iter().any(|&v| height(d, v) > z_min + eps) {
            continue;
        }
        let u = [0, 1, 2].map(|i| f64::from(b[i] - a[i]));
        let w = [0, 1, 2].map(|i| f64::from(c[i] - a[i]));
        let n = [
            u[1] * w[2] - u[2] * w[1],
            u[2] * w[0] - u[0] * w[2],
            u[0] * w[1] - u[1] * w[0],
        ];
        let proj = n[0] * d[0] + n[1] * d[1] + n[2] * d[2];
        if proj <= 0.0 {
            continue;
        }
        area += 0.5 * proj;
        count += 1;
    }
    (area, count)
}

/// `bed_to_z_rotation`: the turn taking `bed_dir` to straight down
/// (Eigen's `Quaternion::FromTwoVectors`).
fn bed_to_z(bed_dir: [f64; 3]) -> Transform {
    let d = unit(bed_dir);
    let target = [0.0, 0.0, -1.0];
    let diff: f64 = (0..3).map(|i| (d[i] - target[i]).powi(2)).sum();
    if diff < 1e-18 {
        return Transform::IDENTITY;
    }
    let c = d[0] * target[0] + d[1] * target[1] + d[2] * target[2];
    // The axis: d × target, or any square to d when they are opposite.
    let mut axis = [
        d[1] * target[2] - d[2] * target[1],
        d[2] * target[0] - d[0] * target[2],
        d[0] * target[1] - d[1] * target[0],
    ];
    let s = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if s < 1e-12 {
        axis = [1.0, 0.0, 0.0];
    } else {
        axis = axis.map(|a| a / s);
    }
    let angle = c.clamp(-1.0, 1.0).acos();
    let (sin, cos) = angle.sin_cos();
    let [x, y, z] = axis;
    let t = 1.0 - cos;
    // Rodrigues: the matrix M with p' = M p; rows of the row-vector layout
    // are M's columns.
    let m = [
        [t * x * x + cos, t * x * y - sin * z, t * x * z + sin * y],
        [t * x * y + sin * z, t * y * y + cos, t * y * z - sin * x],
        [t * x * z - sin * y, t * y * z + sin * x, t * z * z + cos],
    ];
    Transform {
        rows: [
            [m[0][0], m[1][0], m[2][0]],
            [m[0][1], m[1][1], m[2][1]],
            [m[0][2], m[1][2], m[2][2]],
            [0.0; 3],
        ],
    }
}

fn transformed(mesh: &Mesh, t: &Transform) -> Mesh {
    Mesh {
        vertices: mesh
            .vertices
            .iter()
            .map(|v| t.apply(v.map(f64::from)).map(|c| c as f32))
            .collect(),
        triangles: mesh.triangles.clone(),
    }
}

/// `remesh_keep_flat_bottom`: the mesh cut a slab over its base, the part
/// above remeshed, and joined back onto the slab by a union, so the base
/// is the mesh's own. Err: why it was not done, the caller then remeshing
/// it whole.
fn keep_flat_bottom(
    mesh: &Mesh,
    voxel: f64,
    margin: f64,
    bed_dir: [f64; 3],
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<Result<Mesh, &'static str>, Cancelled> {
    let margin = if margin <= 0.0 {
        BOTTOM_MARGIN_VOXELS * voxel
    } else {
        margin
    };
    let to_z = bed_to_z(bed_dir);
    let back = to_z.inverse().unwrap_or(Transform::IDENTITY);
    let work = transformed(mesh, &to_z);
    let (low, high) = work.bounds().expect("not empty");
    let (z_min, z_max) = (low[2], high[2]);
    if z_max <= z_min {
        return Ok(Err("degenerate mesh height"));
    }
    let (area, facets) = bottom_area(&work, [0.0, 0.0, -1.0], BASE_EPS_VOXELS * voxel);
    if facets == 0 || area <= 0.0 {
        return Ok(Err("no flat bottom"));
    }
    if facets < 2 {
        return Ok(Err("bottom is too few triangles"));
    }
    if margin >= f64::from(z_max - z_min) * 0.5 {
        return Ok(Err("bottom margin does not leave an upper part"));
    }
    let cut_at = |z: f32| {
        let z = f64::from(z);
        crate::cut::cut(
            &work,
            &Transform::translation([0.0, 0.0, -z]),
            [0.0, 0.0],
            &Transform::translation([0.0, 0.0, z]),
        )
    };
    let cut_z = z_min + margin as f32;
    let first = cut_at(cut_z);
    if first.upper.is_empty() || first.lower.is_empty() {
        return Ok(Err("cut produced an empty part"));
    }
    let Some(upper) = voxel_remesh(&first.upper.mesh, voxel, cancel)? else {
        return Ok(Err("voxel remesh of the upper part failed"));
    };
    // The slab reaches a voxel into the remeshed part, so the two overlap.
    let slab_cut = cut_at(cut_z + voxel as f32);
    let slab = if slab_cut.lower.is_empty() {
        &first.lower.mesh
    } else {
        &slab_cut.lower.mesh
    };
    let Ok(joined) = crate::boolean::union(&upper, slab) else {
        return Ok(Err(
            "boolean union of the remeshed top and the bottom slab failed",
        ));
    };
    if joined.triangles.is_empty() {
        return Ok(Err(
            "boolean union of the remeshed top and the bottom slab failed",
        ));
    }
    Ok(Ok(transformed(&joined, &back)))
}

/// One sharp edge of the original mesh (`SharpEdge`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SharpEdge {
    pub a: [f32; 3],
    pub b: [f32; 3],
}

fn face_normals(mesh: &Mesh) -> Vec<[f32; 3]> {
    mesh.triangles
        .iter()
        .map(|t| {
            let [a, b, c] = t.map(|i| mesh.vertices[i as usize]);
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * w[2] - u[2] * w[1],
                u[2] * w[0] - u[0] * w[2],
                u[0] * w[1] - u[1] * w[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if len > 1e-12 {
                n.map(|c| c / len)
            } else {
                [0.0, 0.0, 1.0]
            }
        })
        .collect()
}

/// `detect_sharp_edges`: each edge whose two triangles' normals differ by
/// more than `dihedral_deg`, and each edge with one triangle. The
/// reference lists them in the order of a hash map; here by their corners'
/// numbers, so the snapping that takes the first of equally near edges
/// takes the same one every time.
pub fn detect_sharp_edges(mesh: &Mesh, dihedral_deg: f64) -> Vec<SharpEdge> {
    if mesh.triangles.is_empty() {
        return Vec::new();
    }
    let normals = face_normals(mesh);
    let cos = (dihedral_deg * std::f64::consts::PI / 180.0).cos() as f32;
    let mut edges: HashMap<(u32, u32), (i64, i64)> = HashMap::new();
    for (t, tri) in mesh.triangles.iter().enumerate() {
        for e in 0..3 {
            let (a, b) = (tri[e], tri[(e + 1) % 3]);
            let r = edges.entry((a.min(b), a.max(b))).or_insert((-1, -1));
            if r.0 < 0 {
                r.0 = t as i64;
            } else if r.1 < 0 {
                r.1 = t as i64;
            }
        }
    }
    let mut keys: Vec<_> = edges.into_iter().collect();
    keys.sort_unstable_by_key(|(key, _)| *key);
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    keys.into_iter()
        .filter_map(|((lo, hi), (f0, f1))| {
            let sharp = f1 < 0 || dot(normals[f0 as usize], normals[f1 as usize]) < cos;
            if !sharp {
                return None;
            }
            let (a, b) = (mesh.vertices[lo as usize], mesh.vertices[hi as usize]);
            let d2: f32 = (0..3).map(|i| (b[i] - a[i]).powi(2)).sum();
            (d2 >= 1e-18).then_some(SharpEdge { a, b })
        })
        .collect()
}

/// `snap_to_sharp_edges`: each corner within `radius` of a sharp edge
/// moved onto the nearest point of the nearest. Answers how many moved.
pub fn snap_to_sharp_edges(mesh: &mut Mesh, edges: &[SharpEdge], radius: f64) -> usize {
    if mesh.vertices.is_empty() || edges.is_empty() || radius <= 0.0 {
        return 0;
    }
    let r2 = (radius * radius) as f32;
    let cell = radius as f32;
    let cell_of = |p: [f32; 3]| p.map(|c| (c / cell).floor() as i32);
    let mut grid: HashMap<[i32; 3], Vec<usize>> = HashMap::new();
    for (e, edge) in edges.iter().enumerate() {
        let lo: [f32; 3] = std::array::from_fn(|i| edge.a[i].min(edge.b[i]) - cell);
        let hi: [f32; 3] = std::array::from_fn(|i| edge.a[i].max(edge.b[i]) + cell);
        let (c0, c1) = (cell_of(lo), cell_of(hi));
        let cells: i64 = (0..3).map(|i| i64::from(c1[i] - c0[i] + 1)).product();
        if cells > 4096 {
            continue;
        }
        for x in c0[0]..=c1[0] {
            for y in c0[1]..=c1[1] {
                for z in c0[2]..=c1[2] {
                    grid.entry([x, y, z]).or_default().push(e);
                }
            }
        }
    }
    let closest = |e: &SharpEdge, p: [f32; 3]| -> [f32; 3] {
        let ab = [e.b[0] - e.a[0], e.b[1] - e.a[1], e.b[2] - e.a[2]];
        let d2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
        if d2 < 1e-18 {
            return e.a;
        }
        let ap = [p[0] - e.a[0], p[1] - e.a[1], p[2] - e.a[2]];
        let t = ((ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2]) / d2).clamp(0.0, 1.0);
        [e.a[0] + t * ab[0], e.a[1] + t * ab[1], e.a[2] + t * ab[2]]
    };
    let dist2 = |a: [f32; 3], b: [f32; 3]| (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f32>();
    let mut moved = 0;
    for v in &mut mesh.vertices {
        let Some(list) = grid.get(&cell_of(*v)) else {
            continue;
        };
        let (mut best_d2, mut best) = (r2, *v);
        for &e in list {
            let q = closest(&edges[e], *v);
            let d2 = dist2(q, *v);
            if d2 < best_d2 {
                best_d2 = d2;
                best = q;
            }
        }
        if best_d2 < r2 && dist2(best, *v) > 1e-16 {
            *v = best;
            moved += 1;
        }
    }
    moved
}

// ---- the field and its surface ----

const BRICK: i32 = 8;
const BRICK_POINTS: usize = (BRICK * BRICK * BRICK) as usize;

type Key = [i32; 3];

/// The field's values on the band, by bricks of 8 × 8 × 8 grid points;
/// NaN where it has none.
#[derive(Default)]
struct Field {
    bricks: HashMap<Key, Box<[f32; BRICK_POINTS]>>,
}

fn brick_of(p: Key) -> (Key, usize) {
    let b = p.map(|c| c.div_euclid(BRICK));
    let l = p.map(|c| c.rem_euclid(BRICK));
    (b, (l[0] + BRICK * (l[1] + BRICK * l[2])) as usize)
}

impl Field {
    fn get(&self, p: Key) -> Option<f32> {
        let (b, at) = brick_of(p);
        let v = self.bricks.get(&b)?[at];
        (!v.is_nan()).then_some(v)
    }

    /// Its points in order.
    fn points(&self) -> Vec<(Key, f32)> {
        let mut keys: Vec<&Key> = self.bricks.keys().collect();
        keys.sort_unstable();
        let mut out = Vec::new();
        for key in keys {
            let brick = &self.bricks[key];
            for (at, &v) in brick.iter().enumerate() {
                if v.is_nan() {
                    continue;
                }
                let at = at as i32;
                let l = [at % BRICK, (at / BRICK) % BRICK, at / (BRICK * BRICK)];
                out.push(([0, 1, 2].map(|i| key[i] * BRICK + l[i]), v));
            }
        }
        out
    }
}

/// A shell's triangles in index space (millimetres times the voxel scale).
struct Shell {
    triangles: Vec<[[f64; 3]; 3]>,
    low: [f64; 3],
    high: [f64; 3],
}

fn shell_of(mesh: &Mesh, scale: f64) -> Shell {
    let triangles: Vec<[[f64; 3]; 3]> = mesh
        .triangles
        .iter()
        .map(|t| t.map(|i| mesh.vertices[i as usize].map(|c| f64::from(c) * scale)))
        .collect();
    let mut low = [f64::MAX; 3];
    let mut high = [f64::MIN; 3];
    for t in &triangles {
        for p in t {
            for i in 0..3 {
                low[i] = low[i].min(p[i]);
                high[i] = high[i].max(p[i]);
            }
        }
    }
    Shell {
        triangles,
        low,
        high,
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The squared distance from `p` to a triangle (Ericson, 5.1.5).
fn distance2(p: [f64; 3], [a, b, c]: [[f64; 3]; 3]) -> f64 {
    let at = |q: [f64; 3]| dot(sub(p, q), sub(p, q));
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return at(a);
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return at(b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return at([a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]);
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return at(c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return at([a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        let bc = sub(c, b);
        return at([b[0] + w * bc[0], b[1] + w * bc[1], b[2] + w * bc[2]]);
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    at([
        a[0] + ab[0] * v + ac[0] * w,
        a[1] + ab[1] * v + ac[1] * w,
        a[2] + ab[2] * v + ac[2] * w,
    ])
}

/// The distance from each grid point within the band of a shell to it,
/// worked out brick by brick (each brick from the triangles near it, so
/// the bricks can be worked out side by side and come out the same).
fn unsigned_band(shell: &Shell, cancel: &(dyn Fn() -> bool + Sync)) -> Result<Field, Cancelled> {
    let band = f64::from(BAND);
    let mut near: HashMap<Key, Vec<u32>> = HashMap::new();
    for (t, tri) in shell.triangles.iter().enumerate() {
        let lo: Key = std::array::from_fn(|i| {
            ((tri[0][i].min(tri[1][i]).min(tri[2][i]) - band).ceil() as i32).div_euclid(BRICK)
        });
        let hi: Key = std::array::from_fn(|i| {
            ((tri[0][i].max(tri[1][i]).max(tri[2][i]) + band).floor() as i32).div_euclid(BRICK)
        });
        for x in lo[0]..=hi[0] {
            for y in lo[1]..=hi[1] {
                for z in lo[2]..=hi[2] {
                    near.entry([x, y, z]).or_default().push(t as u32);
                }
            }
        }
    }
    let mut keys: Vec<Key> = near.keys().copied().collect();
    keys.sort_unstable();
    let bricks: Vec<Option<(Key, Box<[f32; BRICK_POINTS]>)>> = keys
        .par_iter()
        .map(|key| {
            if cancel() {
                return None;
            }
            let mut d2 = [f64::MAX; BRICK_POINTS];
            for &t in &near[key] {
                let tri = shell.triangles[t as usize];
                let lo: Key = std::array::from_fn(|i| {
                    ((tri[0][i].min(tri[1][i]).min(tri[2][i]) - band).ceil() as i32)
                        .max(key[i] * BRICK)
                });
                let hi: Key = std::array::from_fn(|i| {
                    ((tri[0][i].max(tri[1][i]).max(tri[2][i]) + band).floor() as i32)
                        .min(key[i] * BRICK + BRICK - 1)
                });
                for z in lo[2]..=hi[2] {
                    for y in lo[1]..=hi[1] {
                        for x in lo[0]..=hi[0] {
                            let at = brick_of([x, y, z]).1;
                            let d = distance2([f64::from(x), f64::from(y), f64::from(z)], tri);
                            if d < d2[at] {
                                d2[at] = d;
                            }
                        }
                    }
                }
            }
            let mut out = Box::new([f32::NAN; BRICK_POINTS]);
            let mut any = false;
            for (o, d) in out.iter_mut().zip(d2) {
                if d <= band * band {
                    *o = d.sqrt() as f32;
                    any = true;
                }
            }
            any.then_some((*key, out))
        })
        .collect();
    if cancel() {
        return Err(Cancelled);
    }
    Ok(Field {
        bricks: bricks.into_iter().flatten().collect(),
    })
}

/// Rays along each axis through a shell, as its crossings of every grid
/// line, to tell a point inside by parity. A ray is moved off its grid
/// line by a hair, so it does not run through a corner or along an edge.
struct Rays {
    /// For each axis, each line (by its other two coordinates) and where
    /// along it the shell is crossed, in order.
    lines: [HashMap<[i32; 2], Vec<f64>>; 3],
}

const HAIR: [f64; 2] = [1.234_567e-6, 2.718_281e-6];

impl Rays {
    fn of(shell: &Shell) -> Rays {
        let lines = [0usize, 1, 2].map(|axis| {
            // The line's coordinates (u, v) are the other two axes, in turn.
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut lines: HashMap<[i32; 2], Vec<f64>> = HashMap::new();
            for tri in &shell.triangles {
                let (a, b, c) = (tri[0], tri[1], tri[2]);
                let lo_u = a[u].min(b[u]).min(c[u]).floor() as i32;
                let hi_u = a[u].max(b[u]).max(c[u]).ceil() as i32;
                let lo_v = a[v].min(b[v]).min(c[v]).floor() as i32;
                let hi_v = a[v].max(b[v]).max(c[v]).ceil() as i32;
                for iu in lo_u..=hi_u {
                    for iv in lo_v..=hi_v {
                        let (pu, pv) = (f64::from(iu) + HAIR[0], f64::from(iv) + HAIR[1]);
                        // Barycentric in the (u, v) plane.
                        let e0 = (b[u] - a[u]) * (pv - a[v]) - (b[v] - a[v]) * (pu - a[u]);
                        let e1 = (c[u] - b[u]) * (pv - b[v]) - (c[v] - b[v]) * (pu - b[u]);
                        let e2 = (a[u] - c[u]) * (pv - c[v]) - (a[v] - c[v]) * (pu - c[u]);
                        let inside = (e0 > 0.0 && e1 > 0.0 && e2 > 0.0)
                            || (e0 < 0.0 && e1 < 0.0 && e2 < 0.0);
                        if !inside {
                            continue;
                        }
                        let total = e0 + e1 + e2;
                        // Weights of a, b, c: each the edge opposite it.
                        let (wa, wb, wc) = (e1 / total, e2 / total, e0 / total);
                        let along = wa * a[axis] + wb * b[axis] + wc * c[axis];
                        lines.entry([iu, iv]).or_default().push(along);
                    }
                }
            }
            for list in lines.values_mut() {
                list.sort_unstable_by(f64::total_cmp);
            }
            lines
        });
        Rays { lines }
    }

    /// Whether `p` is inside: two of the three rays through it say so.
    fn inside(&self, p: Key) -> bool {
        let mut votes = 0;
        for axis in 0..3 {
            let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
            let Some(list) = self.lines[axis].get(&[p[u], p[v]]) else {
                continue;
            };
            let below = list.partition_point(|&h| h < f64::from(p[axis]));
            if below % 2 == 1 {
                votes += 1;
            }
        }
        votes >= 2
    }
}

/// The OpenVDB round trip of `remesh_by_voxels`, done here: see the
/// module's words. None when nothing is left.
pub fn voxel_remesh(
    mesh: &Mesh,
    voxel_size: f64,
    cancel: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Mesh>, Cancelled> {
    if mesh.triangles.is_empty() || voxel_size <= 0.0 {
        return Ok(None);
    }
    // `voxel_scale`, a float in the reference.
    let scale = f64::from((1.0 / voxel_size) as f32);
    let mut parts: Vec<Mesh> = crate::split::pieces(mesh)
        .into_iter()
        .filter(|part| part.signed_volume() >= EPSILON)
        .collect();
    if parts.is_empty() {
        parts.push(mesh.clone());
    }
    let shells: Vec<Shell> = parts.iter().map(|part| shell_of(part, scale)).collect();
    let mut bands = Vec::with_capacity(shells.len());
    for shell in &shells {
        bands.push(unsigned_band(shell, cancel)?);
    }
    let rays: Vec<Rays> = shells.par_iter().map(Rays::of).collect();
    if cancel() {
        return Err(Cancelled);
    }
    // Every point of any band, each the least of the shells' values there.
    let mut keys: Vec<Key> = bands
        .iter()
        .flat_map(|band| band.bricks.keys().copied())
        .collect();
    keys.sort_unstable();
    keys.dedup();
    let built: Vec<(Key, Box<[f32; BRICK_POINTS]>)> = keys
        .par_iter()
        .map(|key| {
            let mut out = Box::new([f32::NAN; BRICK_POINTS]);
            for at in 0..BRICK_POINTS {
                let l = [
                    at as i32 % BRICK,
                    (at as i32 / BRICK) % BRICK,
                    at as i32 / (BRICK * BRICK),
                ];
                let p: Key = [0, 1, 2].map(|i| key[i] * BRICK + l[i]);
                let mut value = f32::NAN;
                if !bands.iter().any(|band| band.get(p).is_some()) {
                    continue;
                }
                for (s, band) in bands.iter().enumerate() {
                    let shell = &shells[s];
                    let v = match band.get(p) {
                        Some(u) => {
                            if rays[s].inside(p) {
                                -u
                            } else {
                                u
                            }
                        }
                        None => {
                            let within = (0..3).all(|i| {
                                f64::from(p[i]) >= shell.low[i] && f64::from(p[i]) <= shell.high[i]
                            });
                            if within && rays[s].inside(p) {
                                -BAND
                            } else {
                                BAND
                            }
                        }
                    };
                    value = if value.is_nan() { v } else { value.min(v) };
                }
                out[at] = value;
            }
            (*key, out)
        })
        .collect();
    if cancel() {
        return Err(Cancelled);
    }
    let field = Field {
        bricks: built.into_iter().collect(),
    };
    // A point off every band is inside when it is inside a shell.
    let outside_bands = |p: Key| -> f32 {
        let inside = shells.iter().zip(&rays).any(|(shell, rays)| {
            (0..3).all(|i| f64::from(p[i]) >= shell.low[i] && f64::from(p[i]) <= shell.high[i])
                && rays.inside(p)
        });
        if inside { -BAND } else { BAND }
    };
    let made = surface(&field, &outside_bands, scale);
    Ok((!made.triangles.is_empty()).then_some(made))
}

/// A cell's corner `k` as an offset: bit 0 x, bit 1 y, bit 2 z.
fn corner(k: usize) -> Key {
    [(k & 1) as i32, ((k >> 1) & 1) as i32, ((k >> 2) & 1) as i32]
}

/// The surface of the field at 0 (see the module's words), in millimetres,
/// wound outward. A point off the band takes its value from `beyond`.
///
/// It is found by walking from each grid edge of the band the surface
/// crosses to the crossed edges of the cells round it, so each piece of the
/// surface is followed all the way round and comes out closed, also where
/// the rays' parity is wrong off the band (a hole the rays pass through).
fn surface(field: &Field, beyond: &dyn Fn(Key) -> f32, scale: f64) -> Mesh {
    let mut values: HashMap<Key, f32> = HashMap::new();
    let mut value = |q: Key| -> f32 {
        *values
            .entry(q)
            .or_insert_with(|| field.get(q).unwrap_or_else(|| beyond(q)))
    };
    let inside = |v: f32| v < 0.0;
    // For each cell the surface crosses: the point of each of its twelve
    // edges' patches (u32::MAX: not crossed), by axis and `edge_slot`.
    let mut cells: HashMap<Key, [u32; 12]> = HashMap::new();
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut cell_of =
        |p: Key, value: &mut dyn FnMut(Key) -> f32, vertices: &mut Vec<[f32; 3]>| -> [u32; 12] {
            if let Some(edges) = cells.get(&p) {
                return *edges;
            }
            let mut values = [0.0f32; 8];
            for (k, v) in values.iter_mut().enumerate() {
                let c = corner(k);
                *v = value([p[0] + c[0], p[1] + c[1], p[2] + c[2]]);
            }
            let ins: [bool; 8] = values.map(inside);
            // The corners inside, joined along the cell's edges.
            let mut group = [usize::MAX; 8];
            let mut groups = 0;
            for start in 0..8 {
                if !ins[start] || group[start] != usize::MAX {
                    continue;
                }
                let mut stack = vec![start];
                group[start] = groups;
                while let Some(k) = stack.pop() {
                    for bit in 0..3 {
                        let n = k ^ (1 << bit);
                        if ins[n] && group[n] == usize::MAX {
                            group[n] = groups;
                            stack.push(n);
                        }
                    }
                }
                groups += 1;
            }
            let mut sums = vec![([0.0f64; 3], 0u32); groups];
            let mut of_edge = [usize::MAX; 12];
            for axis in 0..3 {
                for k in 0..8 {
                    if k & (1 << axis) != 0 {
                        continue;
                    }
                    let n = k | (1 << axis);
                    if ins[k] == ins[n] {
                        continue;
                    }
                    let g = if ins[k] { group[k] } else { group[n] };
                    let t = f64::from(values[k]) / (f64::from(values[k]) - f64::from(values[n]));
                    let (ck, cn) = (corner(k), corner(n));
                    for i in 0..3 {
                        sums[g].0[i] += f64::from(p[i] + ck[i]) + t * f64::from(cn[i] - ck[i]);
                    }
                    sums[g].1 += 1;
                    of_edge[axis * 4 + edge_slot(axis, k)] = g;
                }
            }
            let first = vertices.len() as u32;
            for (sum, count) in &sums {
                let mean = sum.map(|c| c / f64::from((*count).max(1)));
                // `to_vec3f(v) / scale`.
                vertices.push(mean.map(|c| (c as f32) / scale as f32));
            }
            let mut edges = [u32::MAX; 12];
            for e in 0..12 {
                if of_edge[e] != usize::MAX {
                    edges[e] = first + of_edge[e] as u32;
                }
            }
            cells.insert(p, edges);
            edges
        };
    // The crossed grid edges to start from: the band's, in order.
    let mut queue: std::collections::VecDeque<(Key, usize)> = std::collections::VecDeque::new();
    let mut seen: std::collections::HashSet<(Key, usize)> = std::collections::HashSet::new();
    for (p, here) in field.points() {
        for axis in 0..3 {
            let mut q = p;
            q[axis] += 1;
            if inside(here) != inside(value(q)) && seen.insert((p, axis)) {
                queue.push_back((p, axis));
            }
        }
    }
    let mut triangles = Vec::new();
    while let Some((p, axis)) = queue.pop_front() {
        let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
        // The four cells round the edge, counter-clockwise about +axis.
        let mut quad = [0u32; 4];
        for (at, (db, dc)) in [(1, 1), (0, 1), (0, 0), (1, 0)].into_iter().enumerate() {
            let mut cell = p;
            cell[b] -= db;
            cell[c] -= dc;
            let mut offset = [0usize; 3];
            offset[b] = db as usize;
            offset[c] = dc as usize;
            let k = offset[0] | (offset[1] << 1) | (offset[2] << 2);
            let edges = cell_of(cell, &mut value, &mut vertices);
            quad[at] = edges[axis * 4 + edge_slot(axis, k)];
            // The cell's other crossed edges, to follow the surface on.
            for e_axis in 0..3 {
                for slot in 0..4 {
                    if edges[e_axis * 4 + slot] == u32::MAX {
                        continue;
                    }
                    let (eb, ec) = ((e_axis + 1) % 3, (e_axis + 2) % 3);
                    let mut from = cell;
                    from[eb] += (slot & 1) as i32;
                    from[ec] += ((slot >> 1) & 1) as i32;
                    if seen.insert((from, e_axis)) {
                        queue.push_back((from, e_axis));
                    }
                }
            }
        }
        // Outward is from inside to outside: along +axis when `p` is in.
        let quad = if inside(value(p)) {
            quad
        } else {
            [quad[3], quad[2], quad[1], quad[0]]
        };
        triangles.push([quad[0], quad[1], quad[2]]);
        triangles.push([quad[2], quad[3], quad[0]]);
    }
    Mesh {
        vertices,
        triangles,
    }
}

/// Which of the four edges along `axis` starts at corner `k` (its bit for
/// `axis` clear): numbered by the other two bits.
fn edge_slot(axis: usize, k: usize) -> usize {
    let (b, c) = ((axis + 1) % 3, (axis + 2) % 3);
    ((k >> b) & 1) | (((k >> c) & 1) << 1)
}

#[cfg(test)]
mod tests;
