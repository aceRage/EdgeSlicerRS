//! Mesh to layer contours: the first stage of the slicing pipeline.

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};
use edge_mesh::Mesh;
use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

mod circle_compensation;
mod elephant_foot;
mod flat;
mod polyholes;
mod rings;

pub use circle_compensation::{
    Circle, CircleCompensation, Marked, approx_circle, auto_circle_compensation, compensation,
};
pub use elephant_foot::compensate_elephant_foot;
pub use polyholes::{
    Margin, Polyholes, RoundHole, Share, create_polyholes, hole_to_polyholes, polyhole_edges,
    round_hole,
};
pub use rings::{Replaced, centroid, same_ring};

mod precise_z;
mod printable;

pub use precise_z::{ADJUSTED_LAYERS, Aligned, align_to_height};
pub use printable::make_overhangs_printable;

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
    slice_with(mesh, planes, None)
}

/// As [`slice`], leaving out of each contour the points where the plane
/// crosses an edge inside a flat face, as the reference does: they are
/// no corners of the model, only of its triangles.
pub fn slice_without_flat_crossings(mesh: &Mesh, planes: &[f64]) -> Vec<Layer> {
    slice_with(mesh, planes, Some(&flat::flat_edges(mesh)))
}

fn slice_with(mesh: &Mesh, planes: &[f64], flat: Option<&FxHashSet<u64>>) -> Vec<Layer> {
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
        .map(|(plane, &z)| slice_layer(mesh, z, &buckets[offsets[plane]..offsets[plane + 1]], flat))
        .collect()
}

/// A solid and, optionally, the volume carved out of it. The carving
/// applies to this body alone, as a negative part applies only to its object.
#[derive(Clone, Copy)]
pub struct Body<'a> {
    pub solid: &'a Mesh,
    pub negative: Option<&'a Mesh>,
    /// Which filament the solid is printed in, counting from 1.
    pub filament: usize,
}

/// One layer's material as regions with holes, ready for perimeters.
#[derive(Clone, Debug)]
pub struct LayerRegions {
    /// Height of the cutting plane in millimetres.
    pub z: f64,
    /// Disjoint regions; overlapping and touching shells are merged.
    pub regions: Vec<ExPolygon>,
    /// The same material shared out among the filaments it is printed in,
    /// in ascending order of filament; empty when there is only one.
    /// Where bodies of different filaments overlap, the higher-numbered
    /// filament has the overlap. (The reference gives it to whichever part
    /// comes later in its object.)
    pub by_filament: Vec<(usize, Vec<ExPolygon>)>,
    /// Each filament's own material as cut, before any of it is shared
    /// out or closed up, in ascending order of filament; empty when there
    /// is only one.
    pub cut_by_filament: Vec<(usize, Vec<ExPolygon>)>,
    /// See [`Layer::open_chains`]; summed over every mesh cut at this height.
    pub open_chains: usize,
}

/// Cuts every body at each plane and resolves the contours into regions:
/// each body's negative volume is removed from that body, everything that
/// overlaps is united, and cracks narrower than twice `closing_radius` are
/// filled (the reference's `slice_closing_radius`; zero leaves them open).
/// `planes` must be sorted ascending.
///
/// The reference closes cracks within one object only. Here closing runs
/// after bodies are united, so two separate objects standing closer than
/// twice the radius are joined as well.
pub fn slice_regions(
    bodies: &[Body<'_>],
    planes: &[f64],
    closing_radius: Coord,
) -> Result<Vec<LayerRegions>, ClipError> {
    slice_regions_with(bodies, planes, closing_radius, slice)
}

/// As [`slice_regions`], cut by [`slice_without_flat_crossings`].
///
/// Closing an outline that has those points on it does not give the
/// outline back: where one lies a unit or two off its flat, growing and
/// shrinking by the closing radius leaves a kink, some hundredths of a
/// millimetre from where the corner beside it was. A round hole cut the
/// plain way is therefore not round enough to be told for one, and the
/// hole options have their layers cut this way.
pub fn slice_regions_without_flat_crossings(
    bodies: &[Body<'_>],
    planes: &[f64],
    closing_radius: Coord,
) -> Result<Vec<LayerRegions>, ClipError> {
    slice_regions_with(bodies, planes, closing_radius, slice_without_flat_crossings)
}

fn slice_regions_with(
    bodies: &[Body<'_>],
    planes: &[f64],
    closing_radius: Coord,
    slice: fn(&Mesh, &[f64]) -> Vec<Layer>,
) -> Result<Vec<LayerRegions>, ClipError> {
    let sliced: Vec<(Vec<Layer>, Option<Vec<Layer>>)> = bodies
        .iter()
        .map(|body| {
            (
                slice(body.solid, planes),
                body.negative.map(|mesh| slice(mesh, planes)),
            )
        })
        .collect();

    let several = bodies
        .iter()
        .any(|body| body.filament != bodies[0].filament);
    planes
        .par_iter()
        .enumerate()
        .map(|(index, &z)| {
            // Contours that need no carving go straight into the final union.
            let mut plain: Vec<&Polygon> = Vec::new();
            let mut carved: Vec<ExPolygon> = Vec::new();
            let mut open_chains = 0;
            // Each filament's own material, kept apart when there are several.
            let mut own: Vec<(usize, Vec<ExPolygon>)> = Vec::new();
            for (body, (solid, negative)) in bodies.iter().zip(&sliced) {
                let solid = &solid[index];
                open_chains += solid.open_chains;
                let negative = negative.as_ref().map(|layers| &layers[index]);
                open_chains += negative.map_or(0, |layer| layer.open_chains);
                let before = carved.len();
                match negative.filter(|layer| !layer.polygons.is_empty()) {
                    None => plain.extend(&solid.polygons),
                    Some(negative) => carved.extend(edge_clip::difference(
                        &edge_clip::union(&solid.polygons)?,
                        &edge_clip::union(&negative.polygons)?,
                    )?),
                }
                if several {
                    let mut material = if carved.len() > before {
                        carved[before..].to_vec()
                    } else {
                        edge_clip::union(&solid.polygons)?
                    };
                    match own
                        .iter_mut()
                        .find(|(filament, _)| *filament == body.filament)
                    {
                        Some((_, regions)) => regions.append(&mut material),
                        None => own.push((body.filament, material)),
                    }
                }
            }
            let united = edge_clip::union(
                plain
                    .into_iter()
                    .chain(carved.iter().flat_map(ExPolygon::rings)),
            )?;
            let regions = if closing_radius > 0 {
                edge_clip::closing(&united, closing_radius, Join::MITER)?
            } else {
                united
            };
            // Shared out from the highest filament down, each taking what
            // no higher one has; then each is let into the cracks that
            // closing filled, so neighbours meet as the whole does.
            own.sort_by_key(|(filament, _)| *filament);
            let mut by_filament: Vec<(usize, Vec<ExPolygon>)> = Vec::with_capacity(own.len());
            let mut taken: Vec<ExPolygon> = Vec::new();
            let mut cut_by_filament = Vec::with_capacity(own.len());
            for (filament, material) in own.into_iter().rev() {
                let mut mine = edge_clip::union_ex(&material)?;
                cut_by_filament.push((filament, mine.clone()));
                if closing_radius > 0 {
                    mine = edge_clip::offset(&mine, closing_radius, Join::MITER)?;
                }
                let mine =
                    edge_clip::difference(&edge_clip::intersection(&mine, &regions)?, &taken)?;
                taken.extend(mine.iter().cloned());
                taken = edge_clip::union_ex(&taken)?;
                by_filament.push((filament, mine));
            }
            by_filament.reverse();
            cut_by_filament.reverse();
            Ok(LayerRegions {
                z,
                regions,
                by_filament,
                cut_by_filament,
                open_chains,
            })
        })
        .collect()
}

/// One triangle's cut: a directed step from one mesh edge to another.
struct Segment {
    from: u64,
    to: u64,
    /// Where the segment starts, on the `from` edge.
    point: Point2,
    /// Another unused segment leaving the same edge, or `NONE`.
    sibling: u32,
}

const NONE: u32 = u32::MAX;

/// Every triangle in `triangles` must cross the plane at `z`. A point cut
/// on an edge in `flat` is left out of its contour.
fn slice_layer(mesh: &Mesh, z: f64, triangles: &[u32], flat: Option<&FxHashSet<u64>>) -> Layer {
    let kept = |edge: u64| flat.is_none_or(|flat| !flat.contains(&edge));
    // Each triangle contributes one segment between two mesh edges, and
    // segments chain by edge identity, so contours close exactly with no
    // distance tolerance. A watertight mesh has one segment leaving each edge.
    // Where shells touch along an edge several leave it and as many arrive,
    // so taking any unused one still closes every contour.
    let mut segments: Vec<Segment> = Vec::with_capacity(triangles.len());
    let mut unused: FxHashMap<u64, u32> =
        FxHashMap::with_capacity_and_hasher(triangles.len(), Default::default());

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
        let sibling = unused.insert(from, segments.len() as u32).unwrap_or(NONE);
        segments.push(Segment {
            from,
            to,
            point: Point2::from_mm(point[0], point[1]),
            sibling,
        });
    }

    // Removes and returns an unused segment leaving `edge`.
    let take = |unused: &mut FxHashMap<u64, u32>, edge: u64| -> Option<&Segment> {
        let head = unused.get_mut(&edge)?;
        let segment = segments.get(*head as usize)?;
        *head = segment.sibling;
        Some(segment)
    };

    // Start contours in triangle order so output is identical run to run and
    // across thread counts; parity checks against the reference depend on that.
    let mut polygons = Vec::new();
    let mut open_chains = 0;
    for start in segments.iter().map(|segment| segment.from) {
        while let Some(first) = take(&mut unused, start) {
            let mut points = vec![first.point];
            let mut to = first.to;
            let closed = loop {
                if to == start {
                    break true;
                }
                match take(&mut unused, to) {
                    Some(segment) => {
                        // Planes through a vertex yield zero-length segments.
                        if points.last() != Some(&segment.point) && kept(segment.from) {
                            points.push(segment.point);
                        }
                        to = segment.to;
                    }
                    None => break false,
                }
            };
            if points.len() > 1 && points.last() == points.first() {
                points.pop();
            }
            // "The seed point is also a triangle junction."
            if closed && !kept(first.from) && !points.is_empty() {
                points.remove(0);
            }
            if !closed {
                open_chains += 1;
            } else if points.len() >= 3 {
                polygons.push(Polygon { points });
            }
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
    fn shells_touching_along_an_edge_both_close() {
        // The vertical edge at x = y = 1 belongs to four triangles.
        let mut soup = cuboid([0.0; 3], [1.0; 3]);
        soup.extend(cuboid([1.0, 1.0, 0.0], [2.0, 2.0, 1.0]));
        let layers = slice(&Mesh::from_triangle_soup(soup), &[0.5]);
        assert_eq!(layers[0].open_chains, 0);
        assert_eq!(areas(&layers[0]).iter().sum::<f64>(), 2.0);
        assert!(areas(&layers[0]).iter().all(|&area| area > 0.0));
    }

    fn shapes(layer: &LayerRegions) -> Vec<(f64, usize)> {
        layer
            .regions
            .iter()
            .map(|region| (region.area_mm2(), region.holes.len()))
            .collect()
    }

    #[test]
    fn regions_pair_holes_with_contours_and_merge_overlaps() {
        let mut hollow = cube(0.0, 10.0);
        hollow.extend(inverted(cube(2.0, 8.0)));
        hollow.extend(cuboid([8.0, 8.0, 0.0], [14.0, 14.0, 10.0]));
        let mesh = Mesh::from_triangle_soup(hollow);
        let body = Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        };
        let layers = slice_regions(&[body], &[1.0, 5.0], 0).unwrap();
        // Two overlapping boxes: 100 + 36 - 4 shared.
        assert_eq!(shapes(&layers[0]), [(132.0, 0)]);
        assert_eq!(shapes(&layers[1]), [(96.0, 1)]);
        assert_eq!(layers[1].z, 5.0);
    }

    #[test]
    fn negative_volume_carves_only_its_own_body() {
        let block = Mesh::from_triangle_soup(cube(0.0, 10.0));
        // Reaches past the block into where the neighbour stands.
        let drill = Mesh::from_triangle_soup(cuboid([4.0, 4.0, 2.0], [16.0, 6.0, 20.0]));
        let neighbour = Mesh::from_triangle_soup(cuboid([12.0, 0.0, 0.0], [20.0, 10.0, 10.0]));
        let bodies = [
            Body {
                solid: &block,
                negative: Some(&drill),
                filament: 1,
            },
            Body {
                solid: &neighbour,
                negative: None,
                filament: 1,
            },
        ];
        let layers = slice_regions(&bodies, &[1.0, 5.0], 0).unwrap();
        // Below the drill both are whole; at z = 5 the block loses a 6 x 2 slot.
        assert_eq!(shapes(&layers[0]), [(100.0, 0), (80.0, 0)]);
        assert_eq!(shapes(&layers[1]), [(88.0, 0), (80.0, 0)]);
        assert!(layers.iter().all(|layer| layer.open_chains == 0));
    }

    #[test]
    fn closing_radius_joins_parts_that_almost_touch() {
        let mut soup = cube(0.0, 10.0);
        soup.extend(cuboid([10.05, 0.0, 0.0], [20.0, 10.0, 10.0]));
        let mesh = Mesh::from_triangle_soup(soup);
        let body = Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        };
        let open = slice_regions(&[body], &[5.0], 0).unwrap();
        assert_eq!(open[0].regions.len(), 2);
        let closed = slice_regions(&[body], &[5.0], edge_geom::scaled(0.049)).unwrap();
        assert_eq!(shapes(&closed[0]), [(200.0, 0)]);
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

#[cfg(test)]
mod filament_tests {
    use edge_geom::unscaled;
    use edge_mesh::primitives::cuboid;

    use super::*;

    fn block(min: [f32; 3], max: [f32; 3]) -> Mesh {
        Mesh::from_triangle_soup(cuboid(min, max))
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    #[test]
    fn one_filament_needs_no_sharing_out() {
        let (a, b) = (
            block([0.0; 3], [10.0; 3]),
            block([20.0, 0.0, 0.0], [30.0, 10.0, 10.0]),
        );
        let bodies = [&a, &b].map(|solid| Body {
            solid,
            negative: None,
            filament: 3,
        });
        let layers = slice_regions(&bodies, &[5.0], 0).unwrap();
        assert_eq!(layers[0].regions.len(), 2);
        assert!(layers[0].by_filament.is_empty());
    }

    #[test]
    fn material_is_shared_out_among_its_filaments() {
        // Two blocks side by side that overlap by 2 mm, and a third apart.
        let left = block([0.0; 3], [10.0; 3]);
        let right = block([8.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let apart = block([40.0, 0.0, 0.0], [50.0, 10.0, 10.0]);
        let bodies = [(&left, 1), (&right, 2), (&apart, 1)].map(|(solid, filament)| Body {
            solid,
            negative: None,
            filament,
        });
        let layers = slice_regions(&bodies, &[5.0], 0).unwrap();
        let layer = &layers[0];
        assert!((area(&layer.regions) - 300.0).abs() < 1e-6);
        // In ascending order of filament, and adding up to the whole.
        assert_eq!(
            layer
                .by_filament
                .iter()
                .map(|(f, _)| *f)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        let (first, second) = (&layer.by_filament[0].1, &layer.by_filament[1].1);
        // The overlap goes to the higher filament.
        assert!((area(second) - 120.0).abs() < 1e-6, "{}", area(second));
        assert!((area(first) - 180.0).abs() < 1e-6, "{}", area(first));
        let reach = first
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|p| unscaled(p.x))
            .filter(|x| *x < 30.0)
            .fold(f64::MIN, f64::max);
        assert!((reach - 8.0).abs() < 1e-6, "filament 1 reaches {reach}");
        assert!(edge_clip::intersection(first, second).unwrap().is_empty());
    }

    #[test]
    fn a_crack_between_filaments_is_closed_for_both() {
        // Blocks 0.05 mm apart: closing fills the crack, and it must not
        // be left belonging to neither.
        let left = block([0.0; 3], [10.0; 3]);
        let right = block([10.05, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let bodies = [(&left, 1), (&right, 2)].map(|(solid, filament)| Body {
            solid,
            negative: None,
            filament,
        });
        let layers = slice_regions(&bodies, &[5.0], edge_geom::scaled(0.049)).unwrap();
        let layer = &layers[0];
        assert_eq!(layer.regions.len(), 1);
        let shared: f64 = layer.by_filament.iter().map(|(_, r)| area(r)).sum();
        assert!((shared - area(&layer.regions)).abs() < 0.02, "{shared}");
    }
}
