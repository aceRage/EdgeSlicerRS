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
    /// Where bodies of one object overlap, the later one has the overlap,
    /// as the reference gives it to whichever part comes later in its
    /// object.
    pub by_filament: Vec<(usize, Vec<ExPolygon>)>,
    /// Each filament's own material as cut, in ascending order of
    /// filament; empty when there is only one. As sliced it is
    /// `by_filament`; it stays as it is where later stages change that.
    pub cut_by_filament: Vec<(usize, Vec<ExPolygon>)>,
    /// See [`Layer::open_chains`]; summed over every mesh cut at this height.
    pub open_chains: usize,
}

/// Cuts every body at each plane and resolves the contours into regions:
/// each body's cracks narrower than twice `closing_radius` are filled (the
/// reference's `slice_closing_radius`; zero leaves them open), its
/// negative volume is removed from it, and bodies that overlap or touch
/// are united. `planes` must be sorted ascending.
///
/// The bodies are taken as the parts of one object
/// ([`slice_regions_by_object`]).
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

/// [`slice_regions_vase`], each part closed by itself and then
/// simplified, as the reference makes each volume's slices
/// (TriangleMeshSlicer.cpp `slice_mesh_ex`, `make_expolygons`: united,
/// grown and shrunk by the closing radius, then `ExPolygon::simplify` by
/// the slicing resolution), and an object's parts then put together as
/// the reference shares them out among its regions (see `object_layer`).
/// Each body is one part; `objects` gives each body's object, and bodies
/// with the same entry are one object's parts, in the object's order; an
/// empty list makes every body one object. Bodies of one object with the
/// same `filament` are one region. Parts and objects standing closer than
/// twice the closing radius are not joined, but where objects overlap
/// they are still united afterwards. `simplify` is the largest a ring may
/// be moved by dropping its points (Douglas-Peucker), zero for none.
pub fn slice_regions_by_object(
    bodies: &[Body<'_>],
    objects: &[Option<usize>],
    planes: &[f64],
    closing_radius: Coord,
    largest_from: &[Option<usize>],
    simplify: Coord,
) -> Result<Vec<LayerRegions>, ClipError> {
    // An object's parts each carry its negative volume: cut once.
    let mut negative_of: Vec<Option<usize>> = Vec::with_capacity(bodies.len());
    let mut negative_meshes: Vec<&Mesh> = Vec::new();
    for body in bodies {
        negative_of.push(body.negative.map(|mesh| {
            match negative_meshes.iter().position(|seen| {
                std::ptr::eq(*seen, mesh)
                    || (seen.triangles == mesh.triangles && seen.vertices == mesh.vertices)
            }) {
                Some(at) => at,
                None => {
                    negative_meshes.push(mesh);
                    negative_meshes.len() - 1
                }
            }
        }));
    }
    let negatives: Vec<Vec<Layer>> = negative_meshes
        .iter()
        .map(|mesh| slice(mesh, planes))
        .collect();
    let sliced: Vec<(Vec<Layer>, Option<&Vec<Layer>>)> = bodies
        .iter()
        .enumerate()
        .map(|(at, body)| {
            let mut solid = slice(body.solid, planes);
            if let Some(Some(from)) = largest_from.get(at) {
                for layer in solid.iter_mut().skip(*from) {
                    keep_largest_contour(&mut layer.polygons);
                }
            }
            (solid, negative_of[at].map(|at| &negatives[at]))
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

/// What the reference unites the parts of one region with: a closing by
/// `EPSILON` (1e-4 mm), which joins parts that touch, or miss each other by
/// less than a fifth of a micron, and nothing wider (PrintObjectSlice.cpp
/// `slices_to_regions`, `closing_ex(expolygons, float(scale_(EPSILON)))`).
const PARTS_MERGE: Coord = 100;

/// What the reference unites a layer's regions with for its outline: a
/// grow and shrink by `ClipperSafetyOffset` (10 units; Layer.cpp
/// `Layer::make_slices`, `union_safety_offset_ex`), so regions that touch
/// are one.
const SAFETY_OFFSET: Coord = 10;

/// One part's cut at one layer as the reference makes a volume's slice
/// (TriangleMeshSlicer.cpp `slice_mesh_ex`, `make_expolygons`): united,
/// grown and shrunk by the closing radius, then simplified.
fn closed_part(
    polygons: &[Polygon],
    closing_radius: Coord,
    simplify: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    if polygons.is_empty() {
        return Ok(Vec::new());
    }
    let united = edge_clip::union(polygons)?;
    let closed = if closing_radius > 0 {
        edge_clip::closing(&united, closing_radius, Join::MITER)?
    } else {
        united
    };
    if simplify > 0 {
        edge_clip::simplify(&closed, simplify)
    } else {
        Ok(closed)
    }
}

/// The box round some regions, `None` for none.
fn extent(regions: &[ExPolygon]) -> Option<[Coord; 4]> {
    let mut points = regions.iter().flat_map(|region| &region.contour.points);
    let first = points.next()?;
    Some(points.fold(
        [first.x, first.y, first.x, first.y],
        |[x0, y0, x1, y1], p| [x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)],
    ))
}

/// One object's layer, from its bodies and their cuts: what
/// [`slice_regions_by_object`] gives for it alone. Each body is a part
/// (volume), in the object's order, and is closed and simplified by
/// itself, its negative volume closed and simplified the same way and
/// taken out of it (PrintObjectSlice.cpp `slice_volumes_inner`). Where two
/// parts overlap, the later one has the overlap: it carves every earlier
/// one (`slices_to_regions`, with `enable_order_independent_overlap_carving`
/// off, which is all this engine follows). The parts of one region are
/// then put together by [`PARTS_MERGE`], and the regions into the layer's
/// outline by [`SAFETY_OFFSET`]. Nothing joins two parts that stand apart:
/// a crack between them stays, of one filament or of two (the reference
/// keeps two parts 0.05 mm apart as two outlines, measured), and neither
/// region reaches into what closing would have filled between them.
fn object_layer<'a, 'b: 'a>(
    members: impl Iterator<Item = (&'a Body<'b>, &'a (Vec<Layer>, Option<&'a Vec<Layer>>))>,
    index: usize,
    several: bool,
    closing_radius: Coord,
    simplify: Coord,
) -> Result<LayerRegions, ClipError> {
    let mut open_chains = 0;
    // Each part's material in the object's order, with its region.
    let mut parts: Vec<(usize, Vec<ExPolygon>)> = Vec::new();
    // Negative volumes already closed, by the cut they came from: an
    // object's parts all carry the same one.
    let mut negatives: Vec<(&Layer, Vec<ExPolygon>)> = Vec::new();
    for (body, (solid, negative)) in members {
        let solid = &solid[index];
        open_chains += solid.open_chains;
        let mut mine = closed_part(&solid.polygons, closing_radius, simplify)?;
        if let Some(negative) = negative.map(|layers| &layers[index]) {
            let seen = (negatives.iter()).position(|(cut, _)| std::ptr::eq(*cut, negative));
            let at = match seen {
                Some(at) => at,
                None => {
                    open_chains += negative.open_chains;
                    let closed = closed_part(&negative.polygons, closing_radius, simplify)?;
                    negatives.push((negative, closed));
                    negatives.len() - 1
                }
            };
            if !mine.is_empty() && !negatives[at].1.is_empty() {
                mine = edge_clip::difference(&mine, &negatives[at].1)?;
            }
        }
        parts.push((body.filament, mine));
    }
    // A later part carves every earlier one it meets.
    if parts.len() > 1 {
        let boxes: Vec<Option<[Coord; 4]>> = parts.iter().map(|(_, own)| extent(own)).collect();
        for later in 1..parts.len() {
            let Some([x0, y0, x1, y1]) = boxes[later] else {
                continue;
            };
            let (earlier, rest) = parts.split_at_mut(later);
            for (at, (_, own)) in earlier.iter_mut().enumerate() {
                let meets = boxes[at]
                    .is_some_and(|[a0, b0, a1, b1]| !(a1 < x0 || a0 > x1 || b1 < y0 || b0 > y1));
                if meets && !own.is_empty() {
                    *own = edge_clip::difference(own, &rest[0].1)?;
                }
            }
        }
    }
    // Each region's parts put together, in ascending order of region.
    let mut by_region: Vec<(usize, Vec<ExPolygon>, usize)> = Vec::new();
    for (key, own) in parts {
        if own.is_empty() {
            continue;
        }
        match by_region.iter_mut().find(|(of, _, _)| *of == key) {
            Some((_, material, count)) => {
                material.extend(own);
                *count += 1;
            }
            None => by_region.push((key, own, 1)),
        }
    }
    let mut by_filament: Vec<(usize, Vec<ExPolygon>)> = Vec::with_capacity(by_region.len());
    for (key, material, count) in by_region {
        let material = if count > 1 {
            edge_clip::closing(&material, PARTS_MERGE, Join::MITER)?
        } else {
            material
        };
        by_filament.push((key, material));
    }
    by_filament.sort_by_key(|(key, _)| *key);
    let regions = match by_filament.as_mut_slice() {
        [] => Vec::new(),
        [(_, only)] if !several => std::mem::take(only),
        [(_, only)] => only.clone(),
        _ => edge_clip::closing(
            &by_filament
                .iter()
                .flat_map(|(_, own)| own.iter().cloned())
                .collect::<Vec<_>>(),
            SAFETY_OFFSET,
            Join::MITER,
        )?,
    };
    if !several {
        by_filament.clear();
    }
    let cut_by_filament = by_filament.clone();
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
        let sorted = |layer: &LayerRegions| {
            let mut all = shapes(layer);
            all.sort_by(|a, b| b.0.total_cmp(&a.0));
            all
        };
        assert_eq!(sorted(&layers[0]), [(100.0, 0), (80.0, 0)]);
        assert_eq!(sorted(&layers[1]), [(88.0, 0), (80.0, 0)]);
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
    fn closing_joins_neither_the_parts_of_one_object_nor_two_objects() {
        // The reference closes each part by itself: two parts 0.05 mm
        // apart stay two outlines (gap1.3mf, measured), in one object as
        // in two.
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
        for objects in [[Some(0), Some(0)], [Some(0), Some(1)]] {
            let layers =
                slice_regions_by_object(&bodies, &objects, &[5.0], radius, &[], 0).unwrap();
            let mut apart = shapes(&layers[0]);
            apart.sort_by(|a, b| a.0.total_cmp(&b.0));
            assert_eq!(apart, [(99.5, 0), (100.0, 0)], "{objects:?}");
        }
        // Parts and objects that overlap are still one region.
        let over = Mesh::from_triangle_soup(cuboid([5.0, 0.0, 0.0], [15.0, 10.0, 10.0]));
        let bodies = [&left, &over].map(|solid| Body {
            solid,
            negative: None,
            filament: 1,
        });
        for objects in [[Some(0), Some(0)], [Some(0), Some(1)]] {
            let both = slice_regions_by_object(&bodies, &objects, &[5.0], radius, &[], 0);
            assert_eq!(shapes(&both.unwrap()[0]), [(150.0, 0)]);
        }
    }

    #[test]
    fn parts_of_one_region_that_touch_are_one() {
        // A face shared by two parts in one filament leaves no seam
        // (`slices_to_regions` closes a region's parts by `EPSILON`).
        let (left, right) = (
            Mesh::from_triangle_soup(cube(0.0, 10.0)),
            Mesh::from_triangle_soup(cuboid([10.0, 0.0, 0.0], [20.0, 10.0, 10.0])),
        );
        let bodies = [&left, &right].map(|solid| Body {
            solid,
            negative: None,
            filament: 1,
        });
        let layers = slice_regions(&bodies, &[5.0], edge_geom::scaled(0.049)).unwrap();
        assert_eq!(shapes(&layers[0]), [(200.0, 0)]);
        assert_eq!(layers[0].regions[0].contour.points.len(), 4);
    }

    #[test]
    fn each_part_is_closed_by_itself() {
        // A notch 0.05 mm wide in one part is closed; the same gap between
        // two parts is not.
        let notched = Mesh::from_triangle_soup({
            let mut soup = cuboid([0.0; 3], [10.0, 4.0, 10.0]);
            soup.extend(cuboid([0.0, 4.05, 0.0], [10.0, 10.0, 10.0]));
            soup.extend(cuboid([9.0, 3.0, 0.0], [10.0, 5.0, 10.0]));
            soup
        });
        let body = Body {
            solid: &notched,
            negative: None,
            filament: 1,
        };
        let layers = slice_regions(&[body], &[5.0], edge_geom::scaled(0.049)).unwrap();
        let closed = shapes(&layers[0]);
        assert_eq!(closed.len(), 1);
        assert!(
            (closed[0].0 - 100.0).abs() < 1e-6 && closed[0].1 == 0,
            "{closed:?}"
        );
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
    fn a_crack_between_filaments_stays_open_for_both() {
        // Blocks 0.05 mm apart: each is closed by itself, so neither
        // reaches into the crack (the reference put both walls 0.21 mm
        // either side of a face two filaments share, measured).
        let left = block([0.0; 3], [10.0; 3]);
        let right = block([10.05, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let bodies = [(&left, 1), (&right, 2)].map(|(solid, filament)| Body {
            solid,
            negative: None,
            filament,
        });
        let layers = slice_regions(&bodies, &[5.0], edge_geom::scaled(0.049)).unwrap();
        let layer = &layers[0];
        assert_eq!(layer.regions.len(), 2);
        assert!((area(&layer.by_filament[0].1) - 100.0).abs() < 1e-9);
        assert!((area(&layer.by_filament[1].1) - 99.5).abs() < 1e-9);
    }

    #[test]
    fn filaments_that_touch_keep_their_faces_and_are_one_outline() {
        let left = block([0.0; 3], [10.0; 3]);
        let right = block([10.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let bodies = [(&left, 1), (&right, 2)].map(|(solid, filament)| Body {
            solid,
            negative: None,
            filament,
        });
        let layers = slice_regions(&bodies, &[5.0], edge_geom::scaled(0.049)).unwrap();
        let layer = &layers[0];
        assert_eq!(layer.regions.len(), 1);
        assert!((area(&layer.regions) - 200.0).abs() < 1e-6);
        for (at, (_, own)) in layer.by_filament.iter().enumerate() {
            assert!((area(own) - 100.0).abs() < 1e-9);
            let xs: Vec<f64> = own[0]
                .contour
                .points
                .iter()
                .map(|p| unscaled(p.x))
                .collect();
            let (low, high) = (
                xs.iter().copied().fold(f64::MAX, f64::min),
                xs.iter().copied().fold(f64::MIN, f64::max),
            );
            assert_eq!((low, high), (10.0 * at as f64, 10.0 * (at + 1) as f64));
        }
    }

    #[test]
    fn the_later_part_has_the_overlap_whatever_its_filament() {
        // The reference carves each earlier part by every later one
        // (`slices_to_regions`), not by filament number.
        let first = block([0.0; 3], [10.0; 3]);
        let second = block([8.0, 0.0, 0.0], [20.0, 10.0, 10.0]);
        let bodies = [(&first, 2), (&second, 1)].map(|(solid, filament)| Body {
            solid,
            negative: None,
            filament,
        });
        let layers = slice_regions(&bodies, &[5.0], 0).unwrap();
        let layer = &layers[0];
        assert!((area(&layer.by_filament[0].1) - 120.0).abs() < 1e-6);
        assert!((area(&layer.by_filament[1].1) - 80.0).abs() < 1e-6);
    }
}
