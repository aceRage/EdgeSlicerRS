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

pub mod layering;
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
/// so a contour represents the layer's average cross-section; the
/// printer's least layer height taken as the layer height (see
/// [`layer_planes_for`]).
pub fn layer_planes(
    z_min: f64,
    z_max: f64,
    first_layer_height: f64,
    layer_height: f64,
) -> Vec<f64> {
    layer_planes_for(z_min, z_max, first_layer_height, layer_height, layer_height)
}

/// Cutting planes at the middle of each layer of an object from `z_min` to
/// `z_max` laid at one layer height: the first layer always, then a layer
/// for as long as its middle is below the object's top, as the reference
/// lays them (Slicing.cpp `generate_object_layers`, over the profile
/// `layer_height_profile_from_ranges` gives an object with no ranges). A
/// layer whose middle falls on the top exactly is not made. How many there
/// are is the reference's own count, its heights added up one by one
/// (`layering::object_layers`, which the printer's least layer height
/// `min_layer_height` plays a part in), so a top that falls within a hair
/// of a layer's middle is decided as there; the planes themselves are
/// worked out from their index, so tall prints do not drift.
pub fn layer_planes_for(
    z_min: f64,
    z_max: f64,
    first_layer_height: f64,
    layer_height: f64,
    min_layer_height: f64,
) -> Vec<f64> {
    assert!(
        first_layer_height > 0.0 && layer_height > 0.0,
        "layer heights must be positive"
    );
    if z_max <= z_min {
        return Vec::new();
    }
    let params = layering::SlicingParams {
        layer_height,
        first_layer_height,
        min_layer_height,
        max_layer_height: layer_height.max(min_layer_height),
        height: z_max - z_min,
    };
    let profile = layering::profile_from_ranges(&params, &[]);
    let count = layering::object_layers(&params, &profile, false).len();
    (0..count)
        .map(|index| {
            if index == 0 {
                z_min + first_layer_height / 2.0
            } else {
                z_min + first_layer_height + (index as f64 - 0.5) * layer_height
            }
        })
        .collect()
}

/// Cuts `mesh` at each plane height. `planes` must be sorted ascending.
///
/// A vertex lying exactly on a plane counts as above it, so every triangle
/// either misses a plane or crosses it through exactly two of its edges, and
/// horizontal faces never produce degenerate contours.
///
/// A contour has no point where the plane crosses an edge inside a flat
/// face, as the reference's has none (TriangleMeshSlicer.cpp
/// `coplanar_edges`): those are corners of the mesh's triangles, not of
/// the model. Left in, they lie a unit or two off their flat, and closing
/// such an outline by `slice_closing_radius` leaves a kink there, some
/// hundredths of a millimetre from where the corner beside it was.
pub fn slice(mesh: &Mesh, planes: &[f64]) -> Vec<Layer> {
    assert!(planes.is_sorted(), "planes must be sorted ascending");
    let flat = flat::flat_edges(mesh);

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
        .map(|(plane, &z)| {
            slice_layer(mesh, z, &buckets[offsets[plane]..offsets[plane + 1]], &flat)
        })
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
/// The bodies are taken as one object: closing joins any two standing
/// closer than twice the radius. [`slice_regions_by_object`] closes each
/// object by itself, as the reference does.
pub fn slice_regions(
    bodies: &[Body<'_>],
    planes: &[f64],
    closing_radius: Coord,
) -> Result<Vec<LayerRegions>, ClipError> {
    slice_regions_vase(bodies, planes, closing_radius, &[])
}

/// [`slice_regions`], with each body cut, from the layer `largest_from`
/// gives it on, to its largest outline alone, turned counter-clockwise:
/// a vase's layers above its bottom, whose holes are closed and whose
/// other islands are dropped (TriangleMeshSlicer.cpp
/// `SlicingMode::PositiveLargestContour`). `largest_from` has an entry
/// for each body, or none at all.
pub fn slice_regions_vase(
    bodies: &[Body<'_>],
    planes: &[f64],
    closing_radius: Coord,
    largest_from: &[Option<usize>],
) -> Result<Vec<LayerRegions>, ClipError> {
    slice_regions_by_object(bodies, &[], planes, closing_radius, largest_from, 0)
}

/// [`slice_regions_vase`], each object closed by itself and then
/// simplified, as the reference makes each object's slices
/// (TriangleMeshSlicer.cpp `slice_mesh_ex`, `make_expolygons`: united,
/// grown and shrunk by the closing radius, then `ExPolygon::simplify` by
/// the slicing resolution). `objects` gives each body's object; bodies
/// with the same entry are one object, closed together, as the parts of
/// one object are in the reference; an empty list makes every body one
/// object. Two objects standing closer than twice the radius are not
/// joined, but where they overlap they are still united afterwards.
/// `simplify` is the largest a ring may be moved by dropping its points
/// (Douglas-Peucker), zero for none.
///
/// The reference closes and simplifies each part (volume) by itself
/// before uniting an object's parts. Here, with `simplify`, each body of
/// an object of several is closed and simplified by itself, as the
/// reference does each part, and the bodies are then united and closed
/// together; an object of one body is closed, then simplified. A body is
/// an object's parts in one filament, which are closed and simplified
/// together. Closing the bodies together again joins an object's parts
/// standing closer than twice the radius, which the reference does not:
/// it keeps two parts 0.05 mm apart in one object, in one filament, as two
/// outlines (measured).
pub fn slice_regions_by_object(
    bodies: &[Body<'_>],
    objects: &[Option<usize>],
    planes: &[f64],
    closing_radius: Coord,
    largest_from: &[Option<usize>],
    simplify: Coord,
) -> Result<Vec<LayerRegions>, ClipError> {
    let sliced: Vec<(Vec<Layer>, Option<Vec<Layer>>)> = bodies
        .iter()
        .enumerate()
        .map(|(at, body)| {
            let mut solid = slice(body.solid, planes);
            if let Some(Some(from)) = largest_from.get(at) {
                for layer in solid.iter_mut().skip(*from) {
                    keep_largest_contour(&mut layer.polygons);
                }
            }
            (solid, body.negative.map(|mesh| slice(mesh, planes)))
        })
        .collect();
    // The objects, each with its bodies, in the order they first come.
    let object_of = |at: usize| objects.get(at).copied().flatten();
    let mut groups: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
    for at in 0..bodies.len() {
        match groups
            .iter_mut()
            .find(|(object, _)| *object == object_of(at))
        {
            Some((_, members)) => members.push(at),
            None => groups.push((object_of(at), vec![at])),
        }
    }

    let several = bodies
        .iter()
        .any(|body| body.filament != bodies[0].filament);
    planes
        .par_iter()
        .enumerate()
        .map(|(index, &z)| {
            let mut open_chains = 0;
            let mut regions = Vec::new();
            let mut by_filament: Vec<(usize, Vec<ExPolygon>)> = Vec::new();
            let mut cut_by_filament: Vec<(usize, Vec<ExPolygon>)> = Vec::new();
            for (_, members) in &groups {
                let object = object_layer(
                    members.iter().map(|&at| (&bodies[at], &sliced[at])),
                    index,
                    several,
                    closing_radius,
                    simplify,
                )?;
                open_chains += object.open_chains;
                regions.extend(object.regions);
                for (into, from) in [
                    (&mut by_filament, object.by_filament),
                    (&mut cut_by_filament, object.cut_by_filament),
                ] {
                    for (filament, mut material) in from {
                        match into.iter_mut().find(|(own, _)| *own == filament) {
                            Some((_, regions)) => regions.append(&mut material),
                            None => into.push((filament, material)),
                        }
                    }
                }
            }
            // Objects closed apart are united where they overlap.
            if groups.len() > 1 {
                regions = edge_clip::union_ex(&regions)?;
                for (_, material) in by_filament.iter_mut().chain(&mut cut_by_filament) {
                    *material = edge_clip::union_ex(material)?;
                }
                by_filament.sort_by_key(|(filament, _)| *filament);
                cut_by_filament.sort_by_key(|(filament, _)| *filament);
            }
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

/// One object's layer, from its bodies and their cuts: what
/// [`slice_regions_by_object`] gives for it alone.
fn object_layer<'a, 'b: 'a>(
    members: impl Iterator<Item = (&'a Body<'b>, &'a (Vec<Layer>, Option<Vec<Layer>>))>,
    index: usize,
    several: bool,
    closing_radius: Coord,
    simplify: Coord,
) -> Result<LayerRegions, ClipError> {
    let members: Vec<_> = members.collect();
    // An object of several bodies has each closed and simplified by
    // itself first, as the reference does each part.
    let apart = simplify > 0 && members.len() > 1;
    // Contours that need no carving go straight into the final union.
    let mut plain: Vec<&Polygon> = Vec::new();
    let mut carved: Vec<ExPolygon> = Vec::new();
    let mut open_chains = 0;
    // Each filament's own material, kept apart when there are several.
    let mut own: Vec<(usize, Vec<ExPolygon>)> = Vec::new();
    for (body, (solid, negative)) in members {
        let solid = &solid[index];
        open_chains += solid.open_chains;
        let negative = negative.as_ref().map(|layers| &layers[index]);
        open_chains += negative.map_or(0, |layer| layer.open_chains);
        let before = carved.len();
        match negative.filter(|layer| !layer.polygons.is_empty()) {
            None if !apart => plain.extend(&solid.polygons),
            None => carved.extend(edge_clip::union(&solid.polygons)?),
            Some(negative) => carved.extend(edge_clip::difference(
                &edge_clip::union(&solid.polygons)?,
                &edge_clip::union(&negative.polygons)?,
            )?),
        }
        if apart {
            let mine = carved.split_off(before);
            let mine = if closing_radius > 0 {
                edge_clip::closing(&mine, closing_radius, Join::MITER)?
            } else {
                mine
            };
            carved.extend(edge_clip::simplify(&mine, simplify)?);
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
    let regions = if simplify > 0 && !apart {
        edge_clip::simplify(&regions, simplify)?
    } else {
        regions
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
        let mine = edge_clip::difference(&edge_clip::intersection(&mine, &regions)?, &taken)?;
        taken.extend(mine.iter().cloned());
        taken = edge_clip::union_ex(&taken)?;
        by_filament.push((filament, mine));
    }
    by_filament.reverse();
    cut_by_filament.reverse();
    Ok(LayerRegions {
        z: 0.0,
        regions,
        by_filament,
        cut_by_filament,
        open_chains,
    })
}

/// Keeps the outline of largest area, holes included in the reckoning,
/// and turns it counter-clockwise (TriangleMeshSlicer.cpp `make_loops`,
/// `PositiveLargestContour`).
fn keep_largest_contour(polygons: &mut Vec<Polygon>) {
    let mut largest: Option<(usize, f64)> = None;
    for (at, polygon) in polygons.iter().enumerate() {
        let area = polygon.signed_area_mm2();
        if largest.is_none_or(|(_, most)| area.abs() > most.abs()) {
            largest = Some((at, area));
        }
    }
    let Some((at, area)) = largest else {
        return;
    };
    let mut kept = polygons.swap_remove(at);
    if area < 0.0 {
        kept.points.reverse();
    }
    polygons.clear();
    polygons.push(kept);
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
fn slice_layer(mesh: &Mesh, z: f64, triangles: &[u32], flat: &FxHashSet<u64>) -> Layer {
    let kept = |edge: u64| !flat.contains(&edge);
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
            // A crossing on a flat edge is never a point of the contour,
            // the first no more than the rest ("The seed point is also a
            // triangle junction"). It is left out as it is met, not taken
            // back afterwards: where the plane passes through a corner
            // of the mesh, the crossings of every edge that meets there
            // are that one point, and taking a flat edge's back would
            // take the corner with it.
            let mut points = Vec::new();
            if kept(first.from) {
                points.push(first.point);
            }
            let mut to = first.to;
            let closed = loop {
                if to == start {
                    break true;
                }
                match take(&mut unused, to) {
                    Some(segment) => {
                        // Planes through a vertex yield zero-length segments.
                        if kept(segment.from) && points.last() != Some(&segment.point) {
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
    fn a_layer_whose_middle_is_the_top_is_not_made() {
        // Slicing.cpp `generate_object_layers`: a layer only while
        // `print_z + height / 2 < top`. A 12 mm part at 0.08 mm over a
        // 0.2 mm first layer: the 149th layer's middle, 12.000, is the top
        // itself, so it ends at 11.96 in 148 layers (a P1S project the
        // reference slices so).
        let planes = layer_planes_for(0.0, 12.0, 0.2, 0.08, 0.08);
        assert_eq!(planes.len(), 148);
        assert!((planes[147] - 11.92).abs() < 1e-9);
        // A hair over the middle and the layer is made.
        assert_eq!(layer_planes_for(0.0, 12.001, 0.2, 0.08, 0.08).len(), 149);
        // Raised off the bed the same.
        assert_eq!(layer_planes_for(3.0, 15.0, 0.2, 0.08, 0.08).len(), 148);
        // The first layer is always laid; nothing for nothing.
        assert_eq!(layer_planes_for(0.0, 0.05, 0.2, 0.2, 0.08).len(), 1);
        assert!(layer_planes_for(1.0, 1.0, 0.2, 0.2, 0.08).is_empty());
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

    #[test]
    fn slices_are_simplified_by_the_tolerance_each_body_by_itself() {
        // A block whose front bows out by 2 um in the middle: within the
        // 2.5 um tolerance, so the bend goes; 10 um is kept.
        let bowed = |bow: f32| {
            edge_mesh::primitives::prism(
                &[(
                    vec![
                        [0.0, 0.0],
                        [5.0, -bow],
                        [10.0, 0.0],
                        [10.0, 10.0],
                        [0.0, 10.0],
                    ],
                    Vec::new(),
                )],
                0.0,
                10.0,
            )
            .unwrap()
        };
        let (slight, marked) = (bowed(0.002), bowed(0.01));
        let tolerance = edge_geom::scaled(0.0025);
        let count = |mesh: &Mesh, simplify: Coord| {
            let body = Body {
                solid: mesh,
                negative: None,
                filament: 1,
            };
            let layers =
                slice_regions_by_object(&[body], &[Some(0)], &[5.0], 0, &[], simplify).unwrap();
            layers[0].regions[0].contour.points.len()
        };
        assert_eq!(count(&slight, 0), 5);
        assert_eq!(count(&slight, tolerance), 4);
        assert_eq!(count(&marked, tolerance), 5);
        // Two bodies of one object are each simplified before they are
        // put together.
        let other = Mesh::from_triangle_soup(cuboid([20.0, 0.0, 0.0], [30.0, 10.0, 10.0]));
        let bodies = [&slight, &other].map(|solid| Body {
            solid,
            negative: None,
            filament: 1,
        });
        let layers =
            slice_regions_by_object(&bodies, &[Some(0), Some(0)], &[5.0], 0, &[], tolerance)
                .unwrap();
        let mut points: Vec<usize> = (layers[0].regions.iter())
            .map(|region| region.contour.points.len())
            .collect();
        points.sort_unstable();
        assert_eq!(points, [4, 4]);
    }

    #[test]
    fn closing_joins_the_parts_of_one_object_and_not_two_objects() {
        let (left, right) = (
            Mesh::from_triangle_soup(cube(0.0, 10.0)),
            Mesh::from_triangle_soup(cuboid([10.05, 0.0, 0.0], [20.0, 10.0, 10.0])),
        );
        let bodies = [&left, &right].map(|solid| Body {
            solid,
            negative: None,
            filament: 1,
        });
        let radius = edge_geom::scaled(0.049);
        let one = slice_regions_by_object(&bodies, &[Some(0), Some(0)], &[5.0], radius, &[], 0);
        assert_eq!(shapes(&one.unwrap()[0]), [(200.0, 0)]);
        let two = slice_regions_by_object(&bodies, &[Some(0), Some(1)], &[5.0], radius, &[], 0);
        let mut apart = shapes(&two.unwrap()[0]);
        apart.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(apart, [(99.5, 0), (100.0, 0)]);
        // Objects that overlap are still one region.
        let over = Mesh::from_triangle_soup(cuboid([5.0, 0.0, 0.0], [15.0, 10.0, 10.0]));
        let bodies = [&left, &over].map(|solid| Body {
            solid,
            negative: None,
            filament: 1,
        });
        let both = slice_regions_by_object(&bodies, &[Some(0), Some(1)], &[5.0], radius, &[], 0);
        assert_eq!(shapes(&both.unwrap()[0]), [(150.0, 0)]);
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
