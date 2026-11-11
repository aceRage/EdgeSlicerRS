//! Triangles painted on a part's surface, for whether a place is near
//! any of them: seams painted on or off.
//!
//! A port of the reference's `GlobalModelInfo::is_enforced` and
//! `is_blocked` (GCode/SeamPlacer.cpp), which ask
//! `AABBTreeIndirect::is_any_triangle_in_radius`.

use crate::{Bvh, Triangles};

/// Painted triangles in a tree of boxes.
#[derive(Clone, Debug)]
pub struct Painted {
    mesh: Triangles,
    tree: Bvh,
}

impl Painted {
    /// None where nothing is painted.
    pub fn new(corners: impl IntoIterator<Item = [[f32; 3]; 3]>) -> Option<Self> {
        let mut mesh = Triangles::default();
        for triangle in corners {
            let first = mesh.vertices.len() as u32;
            mesh.vertices.extend(triangle);
            mesh.indices.push([first, first + 1, first + 2]);
        }
        if mesh.indices.is_empty() {
            return None;
        }
        let tree = Bvh::new(&mesh);
        Some(Self { mesh, tree })
    }

    /// Whether any painted triangle comes nearer `at` than `radius`.
    pub fn within(&self, at: [f32; 3], radius: f32) -> bool {
        let radius = f64::from(radius);
        self.tree
            .closest_within(&self.mesh, at.map(f64::from), radius)
            .is_some_and(|(_, _, distance)| distance < radius * radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_place_is_near_a_painted_triangle_within_the_radius() {
        assert!(Painted::new([]).is_none());
        let painted = Painted::new([
            [[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 10.0]],
            [[20.0, 0.0, 0.0], [30.0, 0.0, 0.0], [20.0, 0.0, 10.0]],
        ])
        .unwrap();
        assert!(painted.within([1.0, 0.3, 1.0], 0.4));
        assert!(!painted.within([1.0, 0.5, 1.0], 0.4));
        // Nearer than the radius, not as near.
        assert!(!painted.within([1.0, 0.5, 1.0], 0.5));
        assert!(painted.within([25.0, -0.2, 2.0], 0.3));
        assert!(!painted.within([15.0, 0.0, 2.0], 4.9));
        assert!(painted.within([15.0, 0.0, 2.0], 5.1));
    }
}
