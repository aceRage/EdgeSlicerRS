//! Mesh to layer contours: the first stage of the slicing pipeline.

use edge_geom::{Point2, Polygon};
use edge_mesh::Mesh;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

#[derive(Clone, Debug)]
pub struct Layer {
    /// Height of the cutting plane in millimetres.
    pub z: f64,
    /// Closed contours with the solid on their left: outer boundaries run
    /// counter-clockwise, holes clockwise.
    pub polygons: Vec<Polygon>,
    /// Contours that never closed. Non-zero means the mesh is not watertight
    /// at this height; the count is a defect indicator, not an exact tally.
    pub open_chains: usize,
}

/// Cutting planes at the middle of each layer, as the reference slicer does,
/// so a contour represents the layer's average cross-section.
pub fn layer_planes(
    z_min: f64,
    z_max: f64,
    first_layer_height: f64,
    layer_height: f64,
) -> Vec<f64> {
    assert!(
        first_layer_height > 0.0 && layer_height > 0.0,
        "layer heights must be positive"
    );
    let mut planes = Vec::new();
    for index in 0.. {
        // Derived from the index rather than accumulated, so tall prints do not drift.
        let plane = if index == 0 {
            z_min + first_layer_height / 2.0
        } else {
            z_min + first_layer_height + (index as f64 - 0.5) * layer_height
        };
        if plane > z_max {
            break;
        }
        planes.push(plane);
    }
    planes
}

/// Cuts `mesh` at each plane height. `planes` must be sorted ascending.
///
/// A vertex lying exactly on a plane counts as above it, so every triangle
/// either misses a plane or crosses it through exactly two of its edges, and
/// horizontal faces never produce degenerate contours.
pub fn slice(mesh: &Mesh, planes: &[f64]) -> Vec<Layer> {
    assert!(planes.is_sorted(), "planes must be sorted ascending");

    // Half-open range of planes each triangle crosses: lowest < plane <= highest.
    let spans: Vec<(u32, u32)> = mesh
        .triangles
        .par_iter()
        .map(|ids| {
            let zs = ids.map(|v| f64::from(mesh.vertices[v as usize][2]));
            let lowest = zs[0].min(zs[1]).min(zs[2]);
            let highest = zs[0].max(zs[1]).max(zs[2]);
            (
                planes.partition_point(|&p| p <= lowest) as u32,
                planes.partition_point(|&p| p <= highest) as u32,
            )
        })
        .collect();

    // Bucket triangle indices per plane in one flat array so layers can be
    // processed independently without each rescanning the whole mesh.
    let mut offsets = vec![0usize; planes.len() + 1];
    for &(first, end) in &spans {
        for plane in first..end {
            offsets[plane as usize + 1] += 1;
        }
    }
    for plane in 0..planes.len() {
        offsets[plane + 1] += offsets[plane];
    }
    let mut cursor = offsets.clone();
    let mut buckets = vec![0u32; offsets[planes.len()]];
    for (triangle, &(first, end)) in spans.iter().enumerate() {
        for plane in first..end {
            buckets[cursor[plane as usize]] = triangle as u32;
            cursor[plane as usize] += 1;
        }
    }

    planes
        .par_iter()
        .enumerate()
        .map(|(plane, &z)| slice_layer(mesh, z, &buckets[offsets[plane]..offsets[plane + 1]]))
        .collect()
}

/// Every triangle in `triangles` must cross the plane at `z`.
fn slice_layer(mesh: &Mesh, z: f64, triangles: &[u32]) -> Layer {
    // Each triangle contributes one directed segment between two mesh edges.
    // Keyed by the edge a segment starts on, a watertight mesh gives every
    // edge exactly one successor, so contours chain exactly with no tolerance.
    let mut next: FxHashMap<u64, (u64, Point2)> =
        FxHashMap::with_capacity_and_hasher(triangles.len(), Default::default());
    let mut starts = Vec::with_capacity(triangles.len());

    for &triangle in triangles {
        let ids = mesh.triangles[triangle as usize];
        let v = ids.map(|id| mesh.vertices[id as usize].map(f64::from));
        let above = v.map(|p| p[2] >= z);

        // Exactly one vertex sits alone on its side of the plane, and the
        // segment joins its two edges. With counter-clockwise winding, leaving
        // through the edge after an above-plane vertex keeps the solid on the
        // left. Deciding this from winding rather than from the segment's
        // direction also holds when the plane passes through a vertex and the
        // segment has zero length.
        let lone = if above[0] == above[1] {
            2
        } else if above[0] == above[2] {
            1
        } else {
            0
        };
        let (after, before) = ((lone + 1) % 3, (lone + 2) % 3);
        let leaving = crossing(ids[lone], v[lone], ids[after], v[after], z);
        let entering = crossing(ids[before], v[before], ids[lone], v[lone], z);
        let ((from, point), (to, _)) = if above[lone] {
            (leaving, entering)
        } else {
            (entering, leaving)
        };
        if next
            .insert(from, (to, Point2::from_mm(point[0], point[1])))
            .is_none()
        {
            starts.push(from);
        }
    }

    // Trace in insertion order so output is identical run to run and across
    // thread counts; parity checks against the reference depend on that.
    let mut polygons = Vec::new();
    let mut open_chains = 0;
    for start in starts {
        let Some((mut to, first)) = next.remove(&start) else {
            continue;
        };
        let mut points = vec![first];
        let closed = loop {
            if to == start {
                break true;
            }
            match next.remove(&to) {
                Some((after, point)) => {
                    // Planes through a vertex yield zero-length segments.
                    if points.last() != Some(&point) {
                        points.push(point);
                    }
                    to = after;
                }
                None => break false,
            }
        };
        if points.len() > 1 && points.last() == points.first() {
            points.pop();
        }
        if !closed {
            open_chains += 1;
        } else if points.len() >= 3 {
            polygons.push(Polygon { points });
        }
    }

    Layer {
        z,
        polygons,
        open_chains,
    }
}

/// Where the edge between two vertices meets the plane, with the edge's key.
fn crossing(a: u32, pa: [f64; 3], b: u32, pb: [f64; 3], z: f64) -> (u64, [f64; 2]) {
    // Always interpolate from the lower vertex id, so the two triangles
    // sharing this edge compute bit-identical points.
    let (low, p_low, high, p_high) = if a < b {
        (a, pa, b, pb)
    } else {
        (b, pb, a, pa)
    };
    let t = (z - p_low[2]) / (p_high[2] - p_low[2]);
    (
        u64::from(low) << 32 | u64::from(high),
        [
            p_low[0] + t * (p_high[0] - p_low[0]),
            p_low[1] + t * (p_high[1] - p_low[1]),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_mesh::primitives::{cuboid, inverted};

    fn cube(min: f32, max: f32) -> Vec<edge_mesh::Triangle> {
        cuboid([min; 3], [max; 3])
    }

    fn areas(layer: &Layer) -> Vec<f64> {
        layer
            .polygons
            .iter()
            .map(Polygon::signed_area_mm2)
            .collect()
    }

    #[test]
    fn planes_sit_mid_layer_and_stop_at_the_top() {
        let planes = layer_planes(0.0, 10.0, 0.2, 0.2);
        assert_eq!(planes.len(), 50);
        assert!((planes[0] - 0.1).abs() < 1e-12 && (planes[49] - 9.9).abs() < 1e-9);

        let planes = layer_planes(5.0, 6.05, 0.3, 0.2);
        assert!((planes[0] - 5.15).abs() < 1e-12 && (planes[1] - 5.4).abs() < 1e-12);
        assert_eq!(planes.len(), 5);
    }

    #[test]
    fn cube_gives_one_outer_contour_per_layer() {
        let mesh = Mesh::from_triangle_soup(cube(0.0, 10.0));
        let layers = slice(&mesh, &layer_planes(0.0, 10.0, 0.2, 0.2));
        assert_eq!(layers.len(), 50);
        for layer in &layers {
            assert_eq!(layer.open_chains, 0);
            assert_eq!(areas(layer), vec![100.0], "z = {}", layer.z);
        }
    }

    #[test]
    fn cavity_comes_out_as_a_clockwise_hole() {
        let mut soup = cube(0.0, 10.0);
        soup.extend(inverted(cube(2.0, 8.0)));
        let layers = slice(&Mesh::from_triangle_soup(soup), &[1.0, 5.0]);
        assert_eq!(areas(&layers[0]), vec![100.0]);
        let mut middle = areas(&layers[1]);
        middle.sort_by(f64::total_cmp);
        assert_eq!(middle, vec![-36.0, 100.0]);
    }

    #[test]
    fn plane_through_vertices_cuts_the_face_below_it() {
        let mesh = Mesh::from_triangle_soup(cube(0.0, 10.0));
        let layers = slice(&mesh, &[0.0, 10.0, 10.5]);
        assert!(layers[0].polygons.is_empty());
        assert_eq!(areas(&layers[1]), vec![100.0]);
        assert_eq!(layers[1].polygons[0].points.len(), 4);
        assert!(layers[2].polygons.is_empty());
    }

    #[test]
    fn missing_face_is_reported_not_closed() {
        let mut soup = cube(0.0, 10.0);
        soup.truncate(10); // removes the +x side
        let layers = slice(&Mesh::from_triangle_soup(soup), &[5.0]);
        assert!(layers[0].polygons.is_empty());
        assert!(layers[0].open_chains > 0);
    }

    #[test]
    fn output_does_not_depend_on_thread_count() {
        let mut soup = Vec::new();
        for i in 0..40 {
            let at = i as f32 * 3.0;
            soup.extend(cuboid([at, 0.0, 0.0], [at + 2.0, 2.0, 1.0 + i as f32]));
        }
        let mesh = Mesh::from_triangle_soup(soup);
        let planes = layer_planes(0.0, 41.0, 0.2, 0.2);
        let run = |threads| {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| slice(&mesh, &planes))
                .into_iter()
                .map(|layer| layer.polygons)
                .collect::<Vec<_>>()
        };
        assert_eq!(run(1), run(8));
    }
}
