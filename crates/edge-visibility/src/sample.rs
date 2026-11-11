//! Points scattered evenly over a mesh's surface: a port of the
//! reference's TriangleSetSampling.cpp (`sample_its_uniform_parallel`),
//! drawn with the same numbers in the same order, so that for the same
//! triangles in the same order they are the same points.

use crate::{Triangles, random::Mt64};

/// Points on a mesh's surface.
#[derive(Clone, Debug, Default)]
pub struct Samples {
    /// The area of the whole surface, in mm².
    pub total_area: f32,
    pub positions: Vec<[f32; 3]>,
    /// Which way the surface faces at each.
    pub normals: Vec<[f32; 3]>,
    /// The triangle each is on.
    pub triangles: Vec<u32>,
}

/// The seed the reference draws its samples with.
const SEED: u64 = 27_644_437;

pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(crate) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(crate) fn norm(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

/// A vector made one long; one of no length is left as it is (Eigen's
/// `normalized`).
pub(crate) fn normalized(a: [f32; 3]) -> [f32; 3] {
    let squared = dot(a, a);
    if squared > 0.0 {
        let length = squared.sqrt();
        a.map(|c| c / length)
    } else {
        a
    }
}

/// `count` points over `mesh`, each triangle getting its share by area.
pub fn sample_uniform(count: usize, mesh: &Triangles) -> Samples {
    let corners =
        |triangle: usize| mesh.indices[triangle].map(|vertex| mesh.vertices[vertex as usize]);
    // Each triangle's area, and the area up to and including it: the
    // running sum is kept in single precision, as it is there.
    let mut area_sum = 0.0f32;
    let mut sums: Vec<f64> = Vec::with_capacity(mesh.indices.len());
    for triangle in 0..mesh.indices.len() {
        let [a, b, c] = corners(triangle);
        let area = 0.5 * f64::from(norm(cross(sub(b, a), sub(c, a))));
        area_sum = (f64::from(area_sum) + area) as f32;
        sums.push(f64::from(area_sum));
    }
    let mut samples = Samples {
        total_area: area_sum,
        ..Samples::default()
    };
    if mesh.indices.is_empty() || area_sum <= 0.0 {
        return samples;
    }
    // The reference keeps a map from each sum to its triangle, in which
    // a triangle of no area shares its sum with the one before and the
    // later of them is the one found: the last triangle of each sum.
    let mut keys: Vec<(f64, u32)> = Vec::with_capacity(sums.len());
    for (triangle, &sum) in sums.iter().enumerate() {
        match keys.last_mut() {
            Some(last) if last.0 == sum => last.1 = triangle as u32,
            _ => keys.push((sum, triangle as u32)),
        }
    }
    let mut engine = Mt64::new(SEED);
    let drawn: Vec<[f64; 3]> = (0..count)
        .map(|_| [engine.unit(), engine.unit(), engine.unit()])
        .collect();
    for [pick, u, v] in drawn {
        let wanted = pick * f64::from(area_sum);
        // The first sum greater than the number drawn (`upper_bound`).
        let found = keys
            .partition_point(|key| key.0 <= wanted)
            .min(keys.len() - 1);
        let triangle = keys[found].1;
        let [a, b, c] = corners(triangle as usize);
        let root = u.sqrt();
        let weights = [
            (1.0 - root) as f32,
            (root * (1.0 - v)) as f32,
            (v * root) as f32,
        ];
        let position = std::array::from_fn(|axis| {
            a[axis] * weights[0] + b[axis] * weights[1] + c[axis] * weights[2]
        });
        samples.positions.push(position);
        samples
            .normals
            .push(normalized(cross(sub(b, a), sub(c, b))));
        samples.triangles.push(triangle);
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_fall_on_the_surface_by_area() {
        // Two triangles in the plane z = 1: one of area 1, one of area 3.
        let mesh = Triangles {
            vertices: vec![
                [0.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [10.0, 0.0, 1.0],
                [12.0, 0.0, 1.0],
                [10.0, 3.0, 1.0],
            ],
            indices: vec![[0, 1, 2], [3, 4, 5]],
        };
        let samples = sample_uniform(4000, &mesh);
        assert_eq!(samples.total_area, 4.0);
        assert_eq!(samples.positions.len(), 4000);
        assert!(samples.positions.iter().all(|p| (p[2] - 1.0).abs() < 1e-6));
        assert!(samples.normals.iter().all(|n| *n == [0.0, 0.0, 1.0]));
        let first = samples.triangles.iter().filter(|t| **t == 0).count();
        assert!((900..1100).contains(&first), "{first}");
        // Each is inside the triangle it is said to be on.
        for (position, triangle) in samples.positions.iter().zip(&samples.triangles) {
            let (x, y) = (position[0] - 10.0 * *triangle as f32, position[1]);
            let height = if *triangle == 0 { 1.0 } else { 3.0 };
            assert!(x >= -1e-5 && y >= -1e-5 && x / 2.0 + y / height <= 1.0 + 1e-5);
        }
        // The same every time, and the first is where the reference's
        // numbers put it: 0.936 of the way through the area is on the
        // second triangle.
        let again = sample_uniform(4000, &mesh);
        assert_eq!(again.positions, samples.positions);
        assert_eq!(samples.triangles[0], 1);
        // Nothing to sample.
        assert!(
            sample_uniform(10, &Triangles::default())
                .positions
                .is_empty()
        );
    }
}
