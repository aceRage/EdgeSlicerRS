//! Colours from a part's sides: which stretches of each layer's outline
//! the painted triangles colour, and the share of the layer each colour
//! takes from them. A port of the side half of the reference's
//! `segmentation_by_painting` (MultiMaterialSegmentation.cpp).
//!
//! What the reference runs, in order, and where it is here:
//!
//! 1. The layer's outline (`input_expolygons`): the regions grown by ten
//!    `SCALED_EPSILON`, united, islands and holes under 0.01 mm² dropped,
//!    shrunk back, simplified and near-duplicate corners taken out.
//!    Matched in `edge_engine::paint::outlines` (each part cut by itself).
//! 2. A grid over the outline's rings (`EdgeGrid::Grid::create`, 10 mm
//!    squares). Here: [`lines::Rings`], a grid of its own; it only finds
//!    candidates, every test made on them is the reference's.
//! 3. Each painted triangle cut by each layer's plane, in single
//!    precision as the reference cuts it (the "Projection of painted
//!    triangles" loop): [`cuts`].
//! 4. A cut matched to the edges of the outline it lies along: less than
//!    30 degrees off and within 50 `SCALED_EPSILON` (5 µm) at one end,
//!    then projected onto the edge (`PaintedLineVisitor`,
//!    `project_line_on_line`): [`lines::painted`].
//! 5. Each edge's painted lines sorted along it and joined where they
//!    overlap or nearly meet (`post_process_painted_lines`,
//!    `filter_painted_lines`): [`lines::post_process`].
//! 6. Each ring cut into lines of one colour, unpainted stretches
//!    coloured 0, short runs given to their neighbours
//!    (`colorize_contours`, `colorize_contour`, `colorize_line`):
//!    [`colour`]. Not yet `filter_colorized_polygon`.
//! 7. A layer all one colour is that colour's whole
//!    (`has_layer_only_one_color`): [`share`].
//! 8. Otherwise the graph of the rings and the Voronoi diagram of their
//!    coloured lines: the diagram's edges that divide two colours kept,
//!    the rest pruned, and each colour's faces traced round it
//!    (`MMU_Graph`, `build_graph`, `remove_multiple_edges_in_vertices`,
//!    `remove_nodes_with_one_arc`, `extract_colored_segments`): [`graph`].
//!
//! After this the reference cuts each colour back to a band along the
//! outline where paint depth is bounded (`cut_segmented_layers`, step 9),
//! takes paint on top and bottom faces through the shell
//! (`segmentation_top_and_bottom_layers`, [`crate::shells`]) and merges
//! the two (`merge_segmented_layers`, [`crate::segment`]).
//!
//! Before this port the engine matched a cut to an edge within 0.15 mm
//! by the nearest cut, gave runs under 0.2 mm to the run before, and
//! united the Voronoi cells of each colour's edges whole. Steps 4 to 8
//! replace that.
//!
//! Left out, as workarounds for flaws or as no-ops:
//! - The Voronoi diagram is not checked and rebuilt rotated when Boost
//!   gets it wrong (`VoronoiDiagram::construct_voronoi`'s
//!   `try_to_repair_if_needed`); a diagram the graph cannot use is an
//!   error for the layer.
//! - A coloured line of no length is dropped before the diagram is made
//!   (Boost's diagram is not defined for one).

pub(crate) mod colour;
pub(crate) mod graph;
pub use graph::SideError;
pub(crate) mod lines;

use edge_geom::{ExPolygon, Point2};

use crate::{Facet, PaintError};

/// A segment from `a` to `b`, in scaled units (the reference's `Line`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Line {
    pub a: Point2,
    pub b: Point2,
}

impl Line {
    pub fn new(a: Point2, b: Point2) -> Self {
        Self { a, b }
    }

    pub fn length(&self) -> f64 {
        dist(self.a, self.b)
    }
}

pub(crate) fn dist2(a: Point2, b: Point2) -> f64 {
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    dx * dx + dy * dy
}

pub(crate) fn dist(a: Point2, b: Point2) -> f64 {
    dist2(a, b).sqrt()
}

/// The reference's `scale_(mm)`: a length in millimetres in scaled
/// units, as a double.
pub(crate) fn scale(mm: f64) -> f64 {
    mm / 1e-6
}

/// `SCALED_EPSILON`, in scaled units.
pub(crate) const SCALED_EPSILON: i64 = 100;

/// A painted triangle cut by one layer's plane: the cut, its colour and
/// which triangle it came from.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cut {
    pub line: Line,
    pub colour: u8,
    pub facet: u32,
}

/// The reference's `is_equal` and `is_less` on single precision.
fn is_equal(left: f32, right: f32) -> bool {
    (left - right).abs() <= 1e-3
}

fn is_less(left: f32, right: f32) -> bool {
    left + 1e-3 < right
}

/// Each layer's cuts through the painted triangles, as the reference
/// makes them (the "Projection of painted triangles" loop of
/// `segmentation_by_painting`): in single precision, a triangle that is
/// flat to within a micrometre left out, and a triangle with a side
/// lying in the plane cut along that side.
pub(crate) fn cuts(facets: &[Facet], planes: &[f64]) -> Vec<Vec<Cut>> {
    const EPSILON: f64 = 1e-4;
    let mut cuts: Vec<Vec<Cut>> = vec![Vec::new(); planes.len()];
    for (index, facet) in facets.iter().enumerate() {
        if facet.filament == 0 {
            continue;
        }
        let mut corners = facet.corners;
        let min_z = corners.iter().map(|c| c[2]).fold(f32::MAX, f32::min);
        let max_z = corners.iter().map(|c| c[2]).fold(f32::MIN, f32::max);
        if is_equal(min_z, max_z) {
            continue;
        }
        corners.sort_by(|a, b| a[2].total_cmp(&b[2]));
        let [p0, p1, p2] = corners;
        // The lowest plane above the triangle's foot, less a tolerance,
        // to the highest one under its top, plus one.
        let low = f64::from((f64::from(min_z) - EPSILON) as f32);
        let high = f64::from((f64::from(max_z) + EPSILON) as f32);
        let first = planes.partition_point(|&z| z <= low);
        let last = planes.partition_point(|&z| z <= high);
        for (layer, &plane) in planes.iter().enumerate().take(last).skip(first) {
            let z = plane as f32;
            if is_less(z, p0[2]) || is_less(p2[2], z) {
                continue;
            }
            let at = |from: [f32; 3], to: [f32; 3], t: f32| {
                [
                    from[0] + t * (to[0] - from[0]),
                    from[1] + t * (to[1] - from[1]),
                ]
            };
            let start = at(p0, p2, (z - p0[2]) / (p2[2] - p0[2]));
            let end = if (is_equal(p0[2], p1[2]) && is_equal(p1[2], z))
                || (is_equal(p1[2], p2[2]) && is_equal(p1[2], z))
            {
                [p1[0], p1[1]]
            } else if f64::from(p1[2]) > plane {
                at(p0, p1, (z - p0[2]) / (p1[2] - p0[2]))
            } else {
                at(p1, p2, (z - p1[2]) / (p2[2] - p1[2]))
            };
            let point = |p: [f32; 2]| Point2 {
                x: scale(f64::from(p[0])).round() as i64,
                y: scale(f64::from(p[1])).round() as i64,
            };
            cuts[layer].push(Cut {
                line: Line::new(point(start), point(end)),
                colour: facet.filament,
                facet: index as u32,
            });
        }
    }
    cuts
}

/// The share of a layer each colour takes from the painted stretches of
/// its outline: one list of regions for every colour, 0 being
/// unpainted. `run_scale` shrinks the lengths runs of colour are
/// filtered by for nozzles under 0.4 mm
/// (`mm_segmentation_run_filter_scale`).
pub(crate) fn share(
    layer: usize,
    outline: &[ExPolygon],
    cuts: &[Cut],
    colours: usize,
    run_scale: f32,
) -> Result<Vec<Vec<ExPolygon>>, PaintError> {
    let mut shares = vec![Vec::new(); colours];
    if outline.is_empty() || cuts.is_empty() {
        return Ok(shares);
    }
    let rings = lines::Rings::new(outline);
    let painted = lines::painted(&rings, cuts);
    if painted.is_empty() {
        return Ok(shares);
    }
    let by_ring = lines::post_process(&rings, painted, run_scale);
    let coloured = colour::colorize_contours(&rings, &by_ring, run_scale);
    // `has_layer_only_one_color`.
    let first = coloured[0][0].colour;
    if coloured.iter().flatten().all(|line| line.colour == first) {
        if let Some(own) = shares.get_mut(first as usize) {
            *own = outline.to_vec();
        }
        return Ok(shares);
    }
    let faces = graph::segments(&coloured, colours)
        .map_err(|source| PaintError::Diagram { layer, source })?;
    for (colour, polygons) in faces.into_iter().enumerate() {
        if !polygons.is_empty() {
            shares[colour] = edge_clip::union(&polygons)?;
        }
    }
    Ok(shares)
}

#[cfg(test)]
mod tests;
