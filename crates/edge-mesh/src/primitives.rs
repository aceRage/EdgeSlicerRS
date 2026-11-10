//! Procedural solids for tests and synthetic benchmarks.

use crate::Triangle;

/// An axis-aligned box as twelve outward-facing triangles.
pub fn cuboid(min: [f32; 3], max: [f32; 3]) -> Vec<Triangle> {
    // Corner `i` takes max on axis `a` when bit `a` of `i` is set.
    let corner = |i: usize| -> [f32; 3] {
        std::array::from_fn(|axis| {
            if i >> axis & 1 == 1 {
                max[axis]
            } else {
                min[axis]
            }
        })
    };
    const FACES: [[usize; 3]; 12] = [
        [0, 2, 3],
        [0, 3, 1], // -z
        [4, 5, 7],
        [4, 7, 6], // +z
        [0, 1, 5],
        [0, 5, 4], // -y
        [2, 6, 7],
        [2, 7, 3], // +y
        [0, 4, 6],
        [0, 6, 2], // -x
        [1, 3, 7],
        [1, 7, 5], // +x
    ];
    FACES.iter().map(|face| face.map(corner)).collect()
}

/// Reverses the winding so a solid reads as a cavity inside another solid.
pub fn inverted(mut triangles: Vec<Triangle>) -> Vec<Triangle> {
    for triangle in &mut triangles {
        triangle.swap(1, 2);
    }
    triangles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuboid_faces_point_outward() {
        let centre = [0.5f32; 3];
        for [a, b, c] in cuboid([0.0; 3], [1.0; 3]) {
            let (u, v) = (sub(b, a), sub(c, a));
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let outward = sub(a, centre);
            let dot: f32 = (0..3).map(|i| normal[i] * outward[i]).sum();
            assert!(dot > 0.0, "triangle {a:?} {b:?} {c:?} faces inward");
        }
    }

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| a[i] - b[i])
    }
}
