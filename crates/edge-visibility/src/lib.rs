//! How much of a part's outside can be seen from each place on its
//! surface, which is where a seam shows least: points scattered over the
//! mesh, rays cast from each to see how much of the sky is hidden from
//! it, and the measure at any place near the surface taken from the
//! points round it.
//!
//! A port of the visibility measure of the reference's SeamPlacer.cpp
//! (`compute_global_occlusion`, `raycast_visibility`,
//! `GlobalModelInfo::calculate_point_visibility`), with what it calls on:
//! TriangleSetSampling.cpp for the points, and ShortEdgeCollapse.cpp for
//! the coarser mesh a fine one is measured as.
//!
//! Everything is the same at any thread count: each sample's rays are
//! its own, and they are gathered in the samples' order.

mod bvh;
mod joints;
mod painted;
mod random;
mod sample;

use rayon::prelude::*;

pub use bvh::{Bvh, Hit};
pub use joints::{JointBody, Joints, part_joints};
pub use painted::Painted;
pub use sample::{Samples, sample_uniform};
use sample::{cross, dot, norm, normalized, sub};

/// Triangles by their corners.
#[derive(Clone, Debug, Default)]
pub struct Triangles {
    pub vertices: Vec<[f32; 3]>,
    pub indices: Vec<[u32; 3]>,
}

/// How many points are scattered over a part
/// (`SeamPlacer::raycasting_visibility_samples_count`).
pub const SAMPLES: usize = 30_000;
/// Rays cast from each point: this many squared
/// (`sqr_rays_per_sample_point`).
const RAYS: usize = 5;

/// How visible each sample is: 1 with open sky all round, less by a
/// 25th for each of its 25 rays that runs into the part from outside.
/// `lean` is the side of the bed seams are leant toward, if any: a
/// sample facing the other way counts as easier seen, by up to 1
/// (`aligned_penalty_direction`). Triangles from `negative_from` on are
/// of volumes cut out of the part. A port of `raycast_visibility`.
pub fn raycast_visibility(
    bvh: &Bvh,
    mesh: &Triangles,
    samples: &Samples,
    negative_from: usize,
    lean: Option<[f32; 3]>,
) -> Vec<f32> {
    // The same directions for every sample, evenly over half a sphere
    // about +Z (`sample_hemisphere_uniform`).
    let step = 1.0f32 / RAYS as f32;
    let mut directions = Vec::with_capacity(RAYS * RAYS);
    for x in 0..RAYS {
        let sample_x = (f64::from(x as f32 * step) + f64::from(step) / 2.0) as f32;
        for y in 0..RAYS {
            let sample_y = (f64::from(y as f32 * step) + f64::from(step) / 2.0) as f32;
            let turn = 2.0 * std::f32::consts::PI * sample_x;
            let out = 2.0 * (sample_y - sample_y * sample_y).sqrt();
            directions.push([
                turn.cos() * out,
                turn.sin() * out,
                (1.0 - 2.0 * sample_y).abs(),
            ]);
        }
    }
    let has_negative = negative_from < mesh.indices.len();
    let face_normal = |triangle: u32| {
        let [a, b, c] =
            mesh.indices[triangle as usize].map(|vertex| mesh.vertices[vertex as usize]);
        normalized(cross(sub(b, a), sub(c, a)))
    };
    let decrease = 1.0 / (RAYS * RAYS) as f32;
    (0..samples.positions.len())
        .into_par_iter()
        .map(|at| {
            let (centre, normal) = (samples.positions[at], samples.normals[at]);
            let mut visibility = 1.0f32;
            if let Some(toward) = lean {
                let away = -dot(normal, toward);
                visibility += ((away + 1.2) * 0.5).clamp(0.0, 1.0);
            }
            // The directions are turned to stand on the surface
            // (`Frame::set_from_z`).
            let z = normalized(normal);
            let aside = if z[0].abs() > 0.99 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            let y = normalized(cross(z, aside));
            let x = cross(y, z);
            let from_negative = samples.triangles[at] as usize >= negative_from;
            for direction in &directions {
                let mut ray: [f32; 3] = std::array::from_fn(|axis| {
                    direction[0] * x[axis] + direction[1] * y[axis] + direction[2] * z[axis]
                });
                // From just above the surface.
                let mut origin: [f64; 3] =
                    std::array::from_fn(|axis| f64::from(centre[axis] + normal[axis] * 0.01));
                if !has_negative {
                    let hit = bvh.first_hit(mesh, origin, ray.map(f64::from));
                    if hit.is_some_and(|hit| dot(face_normal(hit.triangle), ray) <= 0.0) {
                        visibility -= decrease;
                    }
                    continue;
                }
                // With volumes cut out of the part: from a face of one,
                // the ray is cast the other way, from just beneath.
                if from_negative {
                    ray = ray.map(|c| -c);
                    origin =
                        std::array::from_fn(|axis| f64::from(centre[axis] - normal[axis] * 0.01));
                }
                let hits = bvh.all_hits(mesh, origin, ray.map(f64::from));
                if hits.is_empty() {
                    continue;
                }
                // From the last hit back, where the ray is known to be
                // outside everything: in and out of the part, and out
                // and in of what is cut from it.
                let mut inside = 0i32;
                for hit in hits.iter().rev() {
                    let facing = dot(face_normal(hit.triangle), ray);
                    let sign = i32::from(facing > 0.0) - i32::from(facing < 0.0);
                    if hit.triangle as usize >= negative_from {
                        inside -= sign;
                    } else {
                        inside += sign;
                    }
                }
                if inside == 0 {
                    visibility -= decrease;
                }
            }
            visibility
        })
        .collect()
}

/// How visible a part's surface is, to be asked at any place near it.
#[derive(Clone, Debug, Default)]
pub struct Visibility {
    samples: Samples,
    values: Vec<f32>,
    /// How far round a place its samples are taken from, in mm.
    radius: f32,
    grid: Grid,
}

/// The samples by where they are.
#[derive(Clone, Debug, Default)]
struct Grid {
    min: [f32; 3],
    cell: f32,
    size: [usize; 3],
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Grid {
    fn at(&self, position: [f32; 3]) -> [i64; 3] {
        std::array::from_fn(|axis| ((position[axis] - self.min[axis]) / self.cell).floor() as i64)
    }

    fn new(positions: &[[f32; 3]], cell: f32) -> Self {
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for position in positions {
            for axis in 0..3 {
                min[axis] = min[axis].min(position[axis]);
                max[axis] = max[axis].max(position[axis]);
            }
        }
        // No more cells than a few for each sample, however small the
        // radius asked for.
        let mut cell = cell.max(1e-3);
        let cells = |cell: f32| -> [usize; 3] {
            std::array::from_fn(|axis| ((max[axis] - min[axis]) / cell) as usize + 1)
        };
        while cells(cell)
            .iter()
            .map(|count| *count as f64)
            .product::<f64>()
            > 8.0 * positions.len().max(1) as f64
        {
            cell *= 2.0;
        }
        let size = cells(cell);
        let mut grid = Self {
            min,
            cell,
            size,
            starts: vec![0; size[0] * size[1] * size[2] + 1],
            items: vec![0; positions.len()],
        };
        let index = |grid: &Self, position: [f32; 3]| {
            let at = grid.at(position);
            let clamped: [usize; 3] =
                std::array::from_fn(|axis| (at[axis].max(0) as usize).min(grid.size[axis] - 1));
            (clamped[2] * grid.size[1] + clamped[1]) * grid.size[0] + clamped[0]
        };
        for position in positions {
            let cell = index(&grid, *position);
            grid.starts[cell + 1] += 1;
        }
        for cell in 0..grid.starts.len() - 1 {
            grid.starts[cell + 1] += grid.starts[cell];
        }
        let mut next = grid.starts.clone();
        for (sample, position) in positions.iter().enumerate() {
            let cell = index(&grid, *position);
            grid.items[next[cell] as usize] = sample as u32;
            next[cell] += 1;
        }
        grid
    }
}

impl Visibility {
    /// Measures a part: `mesh` is its triangles, those of volumes cut
    /// out of it last, from `negative_from` on. A port of
    /// `compute_global_occlusion`, less the making of the mesh.
    pub fn new(mesh: &Triangles, negative_from: usize, lean: Option<[f32; 3]>) -> Self {
        let samples = sample_uniform(SAMPLES, mesh);
        if samples.positions.is_empty() {
            return Self::default();
        }
        // How far round a place to look so that, nine times in ten,
        // more than four samples are found there.
        let (probability, wanted) = (0.9f32, 4.0f32);
        let density = SAMPLES as f32 / samples.total_area;
        let search_area = wanted / (-probability.ln() * density);
        let radius = (f64::from(search_area) / std::f64::consts::PI).sqrt() as f32;
        let bvh = Bvh::new(mesh);
        let values = raycast_visibility(&bvh, mesh, &samples, negative_from, lean);
        let grid = Grid::new(&samples.positions, radius);
        Self {
            samples,
            values,
            radius,
            grid,
        }
    }

    /// How visible the surface is at a place: the visibility of the
    /// samples within the radius of it, each weighed by how near it is
    /// and how near the place is to its plane; 1 where there is none
    /// (`calculate_point_visibility`).
    pub fn at(&self, position: [f32; 3]) -> f32 {
        if self.values.is_empty() {
            return 1.0;
        }
        let radius = self.radius;
        let low = self.grid.at(position.map(|c| c - radius));
        let high = self.grid.at(position.map(|c| c + radius));
        let size = self.grid.size;
        // Cell by cell and in each cell in the samples' own order, so
        // the sum is the same every time.
        let (mut total_weight, mut total) = (0.0f32, 0.0f32);
        let mut found = false;
        for z in low[2].max(0)..=high[2].min(size[2] as i64 - 1) {
            for y in low[1].max(0)..=high[1].min(size[1] as i64 - 1) {
                for x in low[0].max(0)..=high[0].min(size[0] as i64 - 1) {
                    let cell = (z as usize * size[1] + y as usize) * size[0] + x as usize;
                    let (from, to) = (self.grid.starts[cell], self.grid.starts[cell + 1]);
                    for &sample in &self.grid.items[from as usize..to as usize] {
                        let apart = sub(position, self.samples.positions[sample as usize]);
                        if dot(apart, apart) >= radius * radius {
                            continue;
                        }
                        found = true;
                        let off_plane = dot(apart, self.samples.normals[sample as usize]).abs();
                        let mut weight = radius - off_plane;
                        weight += radius - norm(apart);
                        total += weight * self.values[sample as usize];
                        total_weight += weight;
                    }
                }
            }
        }
        if !found {
            return 1.0;
        }
        total / total_weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box from `min` to `max`, facing out.
    pub(crate) fn block(min: [f32; 3], max: [f32; 3]) -> Triangles {
        let corner = |index: usize| -> [f32; 3] {
            std::array::from_fn(|axis| {
                if index >> axis & 1 == 1 {
                    max[axis]
                } else {
                    min[axis]
                }
            })
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

    pub(crate) fn merged(parts: &[Triangles]) -> Triangles {
        let mut all = Triangles::default();
        for part in parts {
            let offset = all.vertices.len() as u32;
            all.vertices.extend(&part.vertices);
            all.indices
                .extend(part.indices.iter().map(|face| face.map(|v| v + offset)));
        }
        all
    }

    #[test]
    fn a_ray_meets_the_nearest_triangle_ahead_of_it() {
        let mesh = block([0.0; 3], [10.0; 3]);
        let bvh = Bvh::new(&mesh);
        // From outside, at the near face; from inside, at the far one.
        let hit = bvh
            .first_hit(&mesh, [5.0, 5.0, 30.0], [0.0, 0.0, -1.0])
            .unwrap();
        assert_eq!(hit.t, 20.0);
        assert!(hit.triangle == 2 || hit.triangle == 3);
        let hit = bvh
            .first_hit(&mesh, [2.0, 3.0, 5.0], [1.0, 0.0, 0.0])
            .unwrap();
        assert_eq!(hit.t, 8.0);
        // Nothing behind the ray's origin, and nothing off to the side.
        assert_eq!(
            bvh.first_hit(&mesh, [5.0, 5.0, 30.0], [0.0, 0.0, 1.0]),
            None
        );
        assert_eq!(
            bvh.first_hit(&mesh, [50.0, 5.0, 30.0], [0.0, 0.0, -1.0]),
            None
        );
        // All of them, nearest first.
        let hits = bvh.all_hits(&mesh, [5.0, 4.0, 30.0], [0.0, 0.0, -1.0]);
        assert_eq!(
            hits.iter().map(|hit| hit.t).collect::<Vec<_>>(),
            [20.0, 30.0]
        );
        // A mesh of many triangles finds the same as trying every one.
        let many = merged(
            &(0..40)
                .map(|at| {
                    let x = (at % 8) as f32 * 3.0;
                    let y = (at / 8) as f32 * 3.0;
                    block([x, y, 0.0], [x + 2.0, y + 2.0, 1.0 + at as f32 * 0.1])
                })
                .collect::<Vec<_>>(),
        );
        let bvh = Bvh::new(&many);
        for step in 0..200 {
            let origin = [step as f64 * 0.117, 14.0 - step as f64 * 0.071, 9.0];
            let direction = [0.3 - step as f64 * 0.002, -0.2 + step as f64 * 0.003, -1.0];
            let brute = (0..many.indices.len() as u32)
                .filter_map(|triangle| {
                    let corners = many.indices[triangle as usize]
                        .map(|vertex| many.vertices[vertex as usize].map(f64::from));
                    let [a, b, c] = corners;
                    // Möller and Trumbore, plainly.
                    let (e1, e2) = (
                        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
                        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
                    );
                    let p = [
                        direction[1] * e2[2] - direction[2] * e2[1],
                        direction[2] * e2[0] - direction[0] * e2[2],
                        direction[0] * e2[1] - direction[1] * e2[0],
                    ];
                    let det = e1[0] * p[0] + e1[1] * p[1] + e1[2] * p[2];
                    if det.abs() < 1e-9 {
                        return None;
                    }
                    let s = [origin[0] - a[0], origin[1] - a[1], origin[2] - a[2]];
                    let u = (s[0] * p[0] + s[1] * p[1] + s[2] * p[2]) / det;
                    let q = [
                        s[1] * e1[2] - s[2] * e1[1],
                        s[2] * e1[0] - s[0] * e1[2],
                        s[0] * e1[1] - s[1] * e1[0],
                    ];
                    let v = (direction[0] * q[0] + direction[1] * q[1] + direction[2] * q[2]) / det;
                    let t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) / det;
                    (u >= 0.0 && v >= 0.0 && u + v <= 1.0 && t > 0.0).then_some(t as f32)
                })
                .fold(f32::INFINITY, f32::min);
            let found = bvh
                .first_hit(&many, origin, direction)
                .map_or(f32::INFINITY, |hit| hit.t);
            assert_eq!(found, brute, "ray {step}");
        }
        assert_eq!(
            Bvh::new(&Triangles::default()).first_hit(&many, [0.0; 3], [1.0, 0.0, 0.0]),
            None
        );
    }

    #[test]
    fn a_face_in_a_corner_is_seen_less_than_one_in_the_open() {
        // A slab with a block standing on its far end: the slab's top
        // beside the block is partly hidden by it, its top far from the
        // block is open, and the block's own top is open.
        let mesh = merged(&[
            block([0.0, 0.0, 0.0], [40.0, 20.0, 4.0]),
            block([30.0, 0.0, 4.0], [40.0, 20.0, 24.0]),
        ]);
        let visibility = Visibility::new(&mesh, mesh.indices.len(), None);
        let open = visibility.at([5.0, 10.0, 4.0]);
        let corner = visibility.at([29.5, 10.0, 4.0]);
        let top = visibility.at([35.0, 10.0, 24.0]);
        assert!(open > 0.9, "{open}");
        assert!(top > 0.95, "{top}");
        assert!(corner < open - 0.2, "{corner} {open}");
        // Far from the surface there is nothing to go by.
        assert_eq!(visibility.at([5.0, 10.0, 60.0]), 1.0);
        // The same every time, on one thread or many.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let alone = pool.install(|| Visibility::new(&mesh, mesh.indices.len(), None));
        assert_eq!(alone.values, visibility.values);
        // Leant to the back, what faces the front counts as easier
        // seen by 1 and what faces the back by a tenth.
        let leant = Visibility::new(&mesh, mesh.indices.len(), Some([0.0, 1.0, 0.0]));
        let front = leant.at([15.0, 0.0, 2.0]) - visibility.at([15.0, 0.0, 2.0]);
        let back = leant.at([15.0, 20.0, 2.0]) - visibility.at([15.0, 20.0, 2.0]);
        assert!(
            (front - 1.0).abs() < 0.02 && (back - 0.1).abs() < 0.02,
            "{front} {back}"
        );
        // Nothing to measure: everything is in the open.
        assert_eq!(
            Visibility::new(&Triangles::default(), 0, None).at([0.0; 3]),
            1.0
        );
    }

    #[test]
    fn a_volume_cut_out_of_a_part_opens_it_up() {
        // A slab with a block on it, and the same with a volume cut out
        // of the block's near half: the slab's top by the block is then
        // in the open.
        let solid = merged(&[
            block([0.0, 0.0, 0.0], [40.0, 20.0, 4.0]),
            block([20.0, 0.0, 4.0], [40.0, 20.0, 24.0]),
        ]);
        let cut = block([19.0, -1.0, 4.0], [30.0, 21.0, 25.0]);
        let with_cut = merged(&[solid.clone(), cut]);
        let whole = Visibility::new(&solid, solid.indices.len(), None);
        let carved = Visibility::new(&with_cut, solid.indices.len(), None);
        let by_block = [18.5, 10.0, 4.0];
        assert!(whole.at(by_block) < 0.75, "{}", whole.at(by_block));
        assert!(
            carved.at(by_block) > whole.at(by_block) + 0.1,
            "{}",
            carved.at(by_block)
        );
    }
}
