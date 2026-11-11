//! Walls: the loops that trace each region's outline, and the area left
//! inside them for infill.
//!
//! `classic` is a port of the constant-width generator in the reference
//! (`PerimeterGenerator::process_classic`, PerimeterGenerator.cpp), with
//! the gaps between walls filled by lines of whatever width fits. Not
//! ported yet: thin-wall detection, overhang and bridge handling, extra
//! and one-wall-on-top perimeters, fuzzy skin, spiral vase, and the
//! nesting order loops are printed in.

mod settings;

use std::ops::Range;

use edge_clip::{ClipError, Join};
use edge_extrusion::{
    ExtrusionLoop, ExtrusionPath, Flow, FlowError, INSET_OVERLAP_TOLERANCE, Role,
};
use edge_geom::{Coord, ExPolygon, scaled, unscaled};

pub use settings::{ArachneSettings, WallSettings};

/// How much of its spacing an external wall may lose before a narrow island
/// gets a thinner wall instead (PerimeterGenerator.cpp).
const SMALLER_EXT_INSET_OVERLAP_TOLERANCE: f64 = 0.22;
/// Islands shorter than this many mm qualify for the thinner wall.
const NARROW_LOOP_LENGTH_THRESHOLD: f64 = 10.0;
/// Added to an offset so that edges meant to coincide overlap instead
/// (ClipperUtils.hpp `ClipperSafetyOffset`).
const SAFETY_OFFSET: Coord = 10;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error(transparent)]
    Flow(#[from] FlowError),
    #[error("walls of changing width: {0}")]
    Arachne(#[from] edge_arachne::Error),
}

/// Everything the generator needs for one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    pub wall_loops: usize,
    /// The outermost wall.
    pub outer: Flow,
    /// Every wall behind it.
    pub inner: Flow,
    /// Line spacing of solid infill, which sets how far infill sits from the walls.
    pub solid_infill_spacing: Coord,
    /// How far infill reaches into the innermost wall, as a percentage of
    /// the distance between their centre lines.
    pub infill_overlap_percent: f64,
    /// Outline detail finer than this is dropped before walls are placed.
    pub resolution: Coord,
    /// How gaps between walls are filled; none leaves them empty.
    pub gap_fill: Option<GapFill>,
    /// Set where a top surface with more of the part above it elsewhere
    /// keeps only its outer wall (`only_one_wall_top`).
    pub one_wall_top: Option<OneWallTop>,
    /// Set where walls are of changing width; the layer is then walled
    /// by [`arachne`].
    pub arachne: Option<ArachneParams>,
}

/// What walls of changing width are made to on one layer: see
/// `edge_arachne::Params`, which these become.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArachneParams {
    pub min_bead_width: f32,
    pub min_feature_size: f32,
    pub min_length_factor: f32,
    pub wall_transition_length: f32,
    pub wall_transition_angle: f32,
    pub wall_transition_filter_deviation: f32,
    pub wall_distribution_count: i32,
    pub is_top_or_bottom_layer: bool,
    pub wall_maximum_resolution: Coord,
    pub wall_maximum_deviation: Coord,
}

/// What `only_one_wall_top` needs on a layer that is not the last.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OneWallTop {
    /// A top surface narrower than this keeps all its walls
    /// (`min_width_top_surface`).
    pub min_width: Coord,
    /// The sparse infill's line width, which sets how near the outer wall
    /// the top fill may come.
    pub infill_width: Coord,
    /// How far from an overhang a surface is left alone, so that walls
    /// over air are not traded for fill over air.
    pub bridge_margin: Coord,
    /// The number of walls asked for, whatever this layer gets.
    pub wall_loops: usize,
    /// How far the top fill reaches into the outer wall, as a percentage
    /// of the distance between their centre lines.
    pub overlap_percent: f64,
}

/// The part's outlines on the layers next to the one being walled.
#[derive(Clone, Copy, Debug)]
pub struct Around<'a> {
    pub upper: &'a [ExPolygon],
    pub lower: Option<&'a [ExPolygon]>,
}

/// What `split_top_surfaces` makes of the area inside the outer wall.
struct Split {
    /// Where top fill goes in place of inner walls.
    top: Vec<ExPolygon>,
    /// Where the inner walls still go.
    rest: Vec<ExPolygon>,
    /// How near the outer wall the top fill may come.
    clip: Vec<ExPolygon>,
}

/// Divides the area inside the outer wall into what has nothing above it
/// and the rest (PerimeterGenerator.cpp `split_top_surfaces`). None when
/// nothing of it is exposed, which leaves the layer as it was.
fn split_top_surfaces(
    inside: &[ExPolygon],
    around: &Around<'_>,
    params: &Params,
    top: &OneWallTop,
) -> Result<Option<Split>, Error> {
    let ext_width = params.outer.scaled_width();
    let ext_spacing = params.outer.scaled_spacing();
    let (width, spacing) = (params.inner.scaled_width(), params.inner.scaled_spacing());
    // How far the top surface is taken under the walls that stay: as far
    // as the outer wall is wide, where there are inner walls to give up.
    let behind = top.wall_loops.saturating_sub(1) as f64 * spacing as f64;
    let all = if top.wall_loops == 0 {
        0.0
    } else {
        0.9 * (ext_width as f64 + behind)
    };
    let anchor = if all > 0.9 * behind {
        (all - 0.9 * behind) as Coord
    } else {
        0
    };
    // Too thin an exposed strip is not worth a top surface.
    let min_width = top.min_width.max(ext_spacing / 2 + 10);
    let covered = edge_clip::offset(around.upper, min_width, Join::MITER)?;
    // No nearer the outline than a second wall would be.
    let second_wall = edge_clip::offset(inside, -ext_spacing, Join::MITER)?;
    // What hangs over air, and a margin round it, keeps its walls.
    let supported = match around.lower {
        Some(lower) => {
            let over_air = edge_clip::difference(inside, lower)?;
            let reach = ext_spacing.max(width) * 3 / 2 + top.bridge_margin + spacing / 2;
            edge_clip::difference(inside, &edge_clip::offset(&over_air, reach, Join::MITER)?)?
        }
        None => inside.to_vec(),
    };
    let exposed = edge_clip::difference(&supported, &covered)?;
    if exposed.is_empty() {
        return Ok(None);
    }
    let slivers = edge_clip::difference(&exposed, &second_wall)?;
    let rest = edge_clip::difference(
        inside,
        &edge_clip::offset(&exposed, anchor + min_width - ext_spacing / 2, Join::MITER)?,
    )?;
    let top_fill = edge_clip::difference(&second_wall, &rest)?;
    if top_fill.is_empty() {
        return Ok(None);
    }
    let clip = edge_clip::offset(inside, ext_spacing / 2 - top.infill_width / 2, Join::MITER)?;
    let mut rest = edge_clip::intersection(&rest, inside)?;
    if params.gap_fill.is_some() && !slivers.is_empty() {
        rest.extend(slivers);
        rest = edge_clip::union_ex(&rest)?;
    }
    Ok(Some(Split {
        top: top_fill,
        rest,
        clip,
    }))
}

/// What gaps too narrow for a wall are filled with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GapFill {
    /// Supplies the layer height and nozzle; every line takes its width
    /// from the gap it fills. The reference passes the solid infill's flow.
    pub flow: Flow,
    /// Lines shorter than this are not worth printing (`filter_out_gap_fill`).
    pub shortest: Coord,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Perimeters {
    /// Island by island; within an island outermost first, each depth's
    /// outer boundaries before its holes.
    pub loops: Vec<ExtrusionLoop>,
    /// Which of `loops` belong to each island, in order. Islands that got
    /// no walls are not listed.
    pub islands: Vec<Range<usize>>,
    /// Where infill goes, already overlapping the innermost wall.
    pub infill: Vec<ExPolygon>,
    /// Lines down the middle of the gaps between walls, each as wide as
    /// its gap. With walls of changing width there are no gaps, and what
    /// is here are the walls that are not loops of one width: lines down
    /// the middle of thin parts, and loops whose width changes, in their
    /// wall roles.
    pub gap_fill: Vec<ExtrusionPath>,
    /// Gaps left empty because their centre line could not be worked out.
    pub gaps_skipped: usize,
}

/// Concentric constant-width walls for one layer's regions.
pub fn classic(regions: &[ExPolygon], params: &Params) -> Result<Perimeters, Error> {
    classic_between(regions, params, None)
}

/// As [`classic`], knowing the part's outlines on the layers next to this
/// one: where [`Params::one_wall_top`] is set, a top surface among them
/// keeps only its outer wall.
pub fn classic_between(
    regions: &[ExPolygon],
    params: &Params,
    around: Option<Around<'_>>,
) -> Result<Perimeters, Error> {
    if let Some(tuning) = &params.arachne {
        return arachne(regions, params, tuning);
    }
    let ext_width = params.outer.scaled_width();
    let ext_spacing = params.outer.scaled_spacing();
    let spacing = params.inner.scaled_spacing();
    // Centre-line distance between the external wall and the one behind it.
    let ext_spacing2 = scaled(0.5 * (params.outer.spacing + params.inner.spacing));
    // Loops keep their full spacing; this smaller figure only decides when
    // an area is too narrow to take another loop at all.
    let min_spacing = (spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
    let ext_min_spacing_smaller =
        (ext_spacing as f64 * (1.0 - SMALLER_EXT_INSET_OVERLAP_TOLERANCE)) as Coord;
    let narrow_flow = params.outer.with_width(
        params.outer.width - 0.5 * SMALLER_EXT_INSET_OVERLAP_TOLERANCE * params.outer.spacing,
    )?;
    let narrow_area = unscaled(ext_width + ext_min_spacing_smaller) * NARROW_LOOP_LENGTH_THRESHOLD;

    let mut result = Perimeters::default();
    for island in regions {
        let first_loop = result.loops.len();
        // Index of the innermost wall, or -1 for none.
        let mut loop_number = params.wall_loops as isize - 1;
        let mut last = edge_clip::simplify(std::slice::from_ref(island), params.resolution)?;

        // One pass more than there are walls: an area with no room for a
        // further loop is left to gap fill rather than handed to infill.
        let mut gaps: Vec<ExPolygon> = Vec::new();
        // Top fill laid in place of inner walls, and how near the outer
        // wall it may come.
        let (mut top_fills, mut fill_clip): (Vec<ExPolygon>, Vec<ExPolygon>) = Default::default();
        let mut depth: isize = 0;
        while loop_number >= 0 {
            let mut offsets = Vec::new();
            let mut narrow = Vec::new();
            if depth == 0 {
                for piece in &last {
                    let piece = std::slice::from_ref(piece);
                    let fits = edge_clip::offset2(
                        piece,
                        -(ext_width / 2 + ext_min_spacing_smaller / 2),
                        ext_min_spacing_smaller / 2,
                        Join::MITER,
                    )?;
                    // A short island too narrow for a full-width wall still
                    // gets one, slightly thinner.
                    if fits.is_empty() && piece[0].area_mm2() < narrow_area {
                        narrow.extend(edge_clip::offset(
                            piece,
                            -narrow_flow.scaled_width() / 2,
                            Join::MITER,
                        )?);
                    } else {
                        offsets.extend(edge_clip::offset(piece, -ext_width / 2, Join::MITER)?);
                    }
                }
            } else {
                let distance = if depth == 1 { ext_spacing2 } else { spacing };
                // Shrinking past the loop and growing back drops whatever is
                // too narrow to hold it.
                offsets = edge_clip::offset2(
                    &last,
                    -(distance + min_spacing / 2 - 1),
                    min_spacing / 2 - 1,
                    Join::MITER,
                )?;
                // What lies between the last loop and this one and is
                // covered by neither is a gap.
                if params.gap_fill.is_some() {
                    gaps.extend(edge_clip::difference(
                        &edge_clip::offset(&last, -distance / 2, Join::MITER)?,
                        &edge_clip::offset(&offsets, distance / 2 + SAFETY_OFFSET, Join::MITER)?,
                    )?);
                }
            }

            if offsets.is_empty() && narrow.is_empty() {
                loop_number = depth - 1;
                last.clear();
                break;
            }
            if depth > loop_number {
                break;
            }
            let flow = if depth == 0 {
                params.outer
            } else {
                params.inner
            };
            push_loops(&mut result.loops, &offsets, depth as usize, flow);
            push_loops(&mut result.loops, &narrow, depth as usize, narrow_flow);
            last = offsets;
            // With the outer wall down, a top surface inside it gives its
            // inner walls up to top fill: they go round what is left.
            if depth == 0
                && loop_number != 0
                && let (Some(top), Some(around)) = (&params.one_wall_top, &around)
                && let Some(split) = split_top_surfaces(&last, around, params, top)?
            {
                (top_fills, fill_clip, last) = (split.top, split.clip, split.rest);
            }
            depth += 1;
        }

        if result.loops.len() > first_loop {
            result.islands.push(first_loop..result.loops.len());
        }

        if let Some(fill) = &params.gap_fill
            && !gaps.is_empty()
        {
            let filled = fill_gaps(&gaps, fill, params, &mut result.gaps_skipped)?;
            // What a gap line covers is not infilled as well.
            let mut covered = Vec::new();
            for path in &filled {
                covered.extend(edge_clip::thicken(
                    [path.points.as_slice()],
                    path.flow.scaled_width() / 2 + SAFETY_OFFSET,
                )?);
            }
            if !covered.is_empty() {
                last = edge_clip::difference(&last, &edge_clip::union_ex(&covered)?)?;
            }
            result.gap_fill.extend(filled);
        }

        // Infill starts half a line inside the innermost wall's centre
        // line, then reaches back toward it by the configured overlap.
        let mut inset = match loop_number {
            ..0 => 0,
            0 => ext_spacing / 2,
            _ => spacing / 2,
        };
        let mut top_overlap = 0;
        if inset > 0 {
            let reach = unscaled(inset + params.solid_infill_spacing / 2);
            if let Some(top) = &params.one_wall_top {
                top_overlap = scaled(reach * top.overlap_percent / 100.0);
            }
            inset -= scaled(reach * params.infill_overlap_percent / 100.0);
        }
        let min_infill_spacing =
            (params.solid_infill_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
        let mut infill = edge_clip::offset2(
            &edge_clip::simplify(&last, params.resolution)?,
            -inset - min_infill_spacing / 2,
            min_infill_spacing / 2,
            Join::MITER,
        )?;
        // The top fill reaches from its side to the outer wall, half a
        // line nearer than where the inner walls would have started.
        if !top_fills.is_empty() {
            let reached = edge_clip::intersection(
                &fill_clip,
                &edge_clip::offset(&top_fills, ext_spacing / 2, Join::MITER)?,
            )?;
            infill.extend(edge_clip::offset(&reached, top_overlap, Join::MITER)?);
            infill = edge_clip::union_ex(&infill)?;
        }
        result.infill.extend(infill);
    }
    Ok(result)
}

/// A loop's width may differ by this much along it and still be laid as
/// one width, in scaled units: what the reference allows a path before
/// cutting it (`variable_width`).
const LOOP_WIDTH_TOLERANCE: Coord = 50_000;

/// Walls of changing width for one layer's regions: as many as fit
/// wherever the region is measured, up to `wall_loops`
/// (`PerimeterGenerator::process_arachne`, PerimeterGenerator.cpp).
///
/// Not a port of the whole of it. The reference orders the walls itself
/// and keeps a loop a loop whatever its widths; here the loops of one
/// width go to the writer as the classic ones do, to be ordered and
/// given their seams there, and every other wall is a line of its own
/// laid with the fill. One wall over top surfaces, extra walls over
/// overhangs and fuzzy skin are not here.
pub fn arachne(
    regions: &[ExPolygon],
    params: &Params,
    tuning: &ArachneParams,
) -> Result<Perimeters, Error> {
    let ext_width = params.outer.scaled_width();
    let ext_spacing = params.outer.scaled_spacing();
    let spacing = params.inner.scaled_spacing();
    let ext_spacing2 = scaled(0.5 * (params.outer.spacing + params.inner.spacing));
    let height = params.outer.height;
    // A wall's width here is what the reference calls its spacing; the
    // line that lays it is wider by what its rounded edges leave empty.
    let flow_of = |flow: &Flow, width: Coord| {
        flow.with_width(unscaled(width) + height * (1.0 - 0.25 * std::f64::consts::PI))
    };

    let mut result = Perimeters::default();
    for island in regions {
        let first_loop = result.loops.len();
        let simplified = edge_clip::simplify(std::slice::from_ref(island), params.resolution)?;
        // In by what the outer wall is wider than its spacing, so that
        // its outer edge lies on the outline.
        let last = edge_clip::offset(
            &simplified,
            -((ext_width as f64 / 2.0 - ext_spacing as f64 / 2.0) as Coord),
            Join::MITER,
        )?;
        let walls = edge_arachne::generate(
            &last,
            &edge_arachne::Params {
                bead_width_0: ext_spacing,
                bead_width_x: spacing,
                inset_count: params.wall_loops,
                wall_0_inset: 0,
                layer_height: height,
                min_bead_width: tuning.min_bead_width,
                min_feature_size: tuning.min_feature_size,
                min_length_factor: tuning.min_length_factor,
                wall_transition_length: tuning.wall_transition_length,
                wall_transition_angle: tuning.wall_transition_angle,
                wall_transition_filter_deviation: tuning.wall_transition_filter_deviation,
                wall_distribution_count: tuning.wall_distribution_count,
                is_top_or_bottom_layer: tuning.is_top_or_bottom_layer,
                wall_maximum_resolution: tuning.wall_maximum_resolution,
                wall_maximum_deviation: tuning.wall_maximum_deviation,
            },
        )?;

        let mut lines = Vec::new();
        for depth in &walls.toolpaths {
            let mut holes = Vec::new();
            for wall in depth {
                if wall.junctions.len() < 2 {
                    continue;
                }
                let (role, flow) = if wall.inset_idx == 0 {
                    (Role::ExternalPerimeter, &params.outer)
                } else {
                    (Role::Perimeter, &params.inner)
                };
                let widths = wall.junctions.iter().map(|junction| junction.w);
                let (least, most) = (widths.clone().min().unwrap_or(0), widths.max().unwrap_or(0));
                if wall.is_closed
                    && most - least <= LOOP_WIDTH_TOLERANCE
                    && wall.junctions.len() > 3
                {
                    let is_hole = !wall.is_contour();
                    let mut polygon = wall.to_polygon();
                    // A closed wall ends on the point it began at.
                    if polygon.points.len() > 1 && polygon.points.first() == polygon.points.last() {
                        polygon.points.pop();
                    }
                    // Counter-clockwise round material, clockwise round a hole.
                    if polygon.is_ccw() == is_hole {
                        polygon.points.reverse();
                    }
                    let made = ExtrusionLoop {
                        polygon,
                        role,
                        is_hole,
                        depth: wall.inset_idx,
                        flow: flow_of(flow, wall.junctions[0].w)?,
                    };
                    if is_hole {
                        holes.push(made);
                    } else {
                        result.loops.push(made);
                    }
                    continue;
                }
                // Two widths a segment: where it starts and where it ends.
                let line = edge_medial::ThickPolyline {
                    points: wall.junctions.iter().map(|junction| junction.p).collect(),
                    widths: wall
                        .junctions
                        .windows(2)
                        .flat_map(|pair| [pair[0].w as f64, pair[1].w as f64])
                        .collect(),
                    open_ends: (!wall.is_closed, !wall.is_closed),
                };
                lines.extend(edge_medial::variable_width(&[line], role, flow));
            }
            result.loops.extend(holes);
        }
        if result.loops.len() > first_loop {
            result.islands.push(first_loop..result.loops.len());
        }
        result.gap_fill.extend(lines);

        // Infill reaches from the inside of the innermost wall back
        // into it by the overlap set.
        let mut infill_contour = walls.inner_contour;
        let wall_count = walls.toolpaths.len();
        let inside_spacing = if wall_count == 1 {
            ext_spacing2
        } else {
            spacing
        };
        if edge_clip::offset(&infill_contour, -inside_spacing / 2, Join::MITER)?.is_empty() {
            infill_contour.clear();
        }
        let inset = match wall_count {
            0 => 0,
            1 => ext_spacing,
            _ => spacing,
        };
        let inset = scaled(unscaled(inset) * params.infill_overlap_percent / 100.0);
        let min_infill_spacing =
            (params.solid_infill_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
        result.infill.extend(edge_clip::offset2(
            &edge_clip::simplify(&infill_contour, params.resolution)?,
            -min_infill_spacing / 2,
            inset + min_infill_spacing / 2,
            Join::MITER,
        )?);
    }
    Ok(result)
}

/// Lines down the middle of whatever part of `gaps` is too narrow for a
/// wall and wide enough to print (PerimeterGenerator.cpp, "fill gaps").
fn fill_gaps(
    gaps: &[ExPolygon],
    fill: &GapFill,
    params: &Params,
    skipped: &mut usize,
) -> Result<Vec<ExtrusionPath>, Error> {
    let thinnest = 0.2
        * params.inner.scaled_width().min(params.outer.scaled_width()) as f64
        * (1.0 - INSET_OVERLAP_TOLERANCE);
    let widest = 2.0 * params.inner.scaled_spacing() as f64;
    // Without the slivers too thin to print and the parts a wall could
    // have had.
    let narrow = edge_clip::difference(
        &edge_clip::opening(gaps, (thinnest / 2.0) as Coord, Join::MITER)?,
        &edge_clip::offset2(
            gaps,
            -(widest / 2.0) as Coord,
            (widest / 2.0) as Coord + SAFETY_OFFSET,
            Join::MITER,
        )?,
    )?;
    let mut lines = Vec::new();
    for gap in &narrow {
        for gap in edge_clip::simplify(std::slice::from_ref(gap), params.resolution)? {
            match edge_medial::medial_axis(&gap, thinnest, widest) {
                Ok(found) => lines.extend(found),
                Err(_) => *skipped += 1,
            }
        }
    }
    lines.retain(|line| line.length() >= fill.shortest as f64);
    Ok(edge_medial::variable_width(
        &lines,
        Role::GapFill,
        &fill.flow,
    ))
}

/// One depth's loops: every outer boundary, then every hole.
fn push_loops(loops: &mut Vec<ExtrusionLoop>, shells: &[ExPolygon], depth: usize, flow: Flow) {
    let role = if depth == 0 {
        Role::ExternalPerimeter
    } else {
        Role::Perimeter
    };
    let contours = shells.iter().map(|shell| (&shell.contour, false));
    let holes = shells
        .iter()
        .flat_map(|shell| &shell.holes)
        .map(|hole| (hole, true));
    for (polygon, is_hole) in contours.chain(holes) {
        loops.push(ExtrusionLoop {
            polygon: polygon.clone(),
            role,
            is_hole,
            depth,
            flow,
        });
    }
}

#[cfg(test)]
mod tests {
    use edge_geom::{Point2, Polygon};

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: rect(x0, y0, x1, y1),
            holes: Vec::new(),
        }
    }

    /// Reference defaults on a 0.4 mm nozzle at 0.2 mm layers: every line
    /// 0.45 mm wide, 0.40708 mm apart.
    fn params(wall_loops: usize) -> Params {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        Params {
            wall_loops,
            outer: flow,
            inner: flow,
            solid_infill_spacing: flow.scaled_spacing(),
            infill_overlap_percent: 15.0,
            one_wall_top: None,
            resolution: scaled(0.01),
            gap_fill: None,
            arachne: None,
        }
    }

    /// Side length in mm of a loop that is an axis-aligned square.
    fn side(wall: &ExtrusionLoop) -> f64 {
        assert_eq!(wall.polygon.points.len(), 4);
        wall.length_mm() / 4.0
    }

    #[test]
    fn a_top_surface_below_the_last_layer_keeps_only_its_outer_wall() {
        // A 40 x 20 mm layer whose left half has more of the part on it
        // and whose right half is the part's top there.
        let layer = [block(0.0, 0.0, 40.0, 20.0)];
        let (upper, lower) = ([block(0.0, 0.0, 20.0, 20.0)], [block(0.0, 0.0, 40.0, 20.0)]);
        let top = OneWallTop {
            min_width: scaled(0.9),
            infill_width: scaled(0.45),
            bridge_margin: scaled(1.0),
            wall_loops: 3,
            overlap_percent: 25.0,
        };
        let with = Params {
            one_wall_top: Some(top),
            ..params(3)
        };
        let between = Around {
            upper: &upper,
            lower: Some(&lower),
        };
        let thinned = classic_between(&layer, &with, Some(between)).unwrap();
        let plain = classic(&layer, &params(3)).unwrap();
        // One outer wall round the whole layer, as before.
        let outer = |walls: &Perimeters| -> Vec<f64> {
            walls
                .loops
                .iter()
                .filter(|wall| wall.depth == 0)
                .map(ExtrusionLoop::length_mm)
                .collect()
        };
        assert_eq!(outer(&thinned), outer(&plain));
        // The inner walls go round the covered half only.
        fn inner(walls: &Perimeters) -> Vec<&ExtrusionLoop> {
            walls.loops.iter().filter(|wall| wall.depth > 0).collect()
        }
        assert_eq!(inner(&thinned).len(), 2);
        let furthest = inner(&thinned)
            .iter()
            .flat_map(|wall| &wall.polygon.points)
            .map(|point| point.x)
            .max()
            .unwrap();
        // They stop a little short of where the part above ends: the top
        // fill runs that far under it, to have something to hold on to.
        assert!(
            furthest > scaled(19.0) && furthest < scaled(20.0),
            "{}",
            unscaled(furthest)
        );
        // Fill takes their place: it reaches to within a wall or so of
        // the outline on the exposed side, where it stopped three walls in.
        let reach = |walls: &Perimeters| {
            walls
                .infill
                .iter()
                .flat_map(|area| &area.contour.points)
                .map(|point| point.x)
                .max()
                .unwrap()
        };
        assert!(
            reach(&thinned) > scaled(39.4),
            "{}",
            unscaled(reach(&thinned))
        );
        assert!(reach(&plain) < scaled(39.0), "{}", unscaled(reach(&plain)));
        let area = |walls: &Perimeters| walls.infill.iter().map(ExPolygon::area_mm2).sum::<f64>();
        assert!(area(&thinned) > area(&plain) + 10.0);

        // With the whole layer covered, or no neighbours known, nothing changes.
        let covered = [block(0.0, 0.0, 40.0, 20.0)];
        let all = Around {
            upper: &covered,
            lower: Some(&lower),
        };
        assert_eq!(classic_between(&layer, &with, Some(all)).unwrap(), plain);
        assert_eq!(classic_between(&layer, &with, None).unwrap(), plain);
        // A strip of top narrower than the least width keeps its walls.
        let nearly = [block(0.0, 0.0, 39.5, 20.0)];
        let sliver = Around {
            upper: &nearly,
            lower: Some(&lower),
        };
        assert_eq!(classic_between(&layer, &with, Some(sliver)).unwrap(), plain);
        // What hangs over air keeps its walls too.
        let short = [block(0.0, 0.0, 20.0, 20.0)];
        let over_air = Around {
            upper: &upper,
            lower: Some(&short),
        };
        assert_eq!(
            inner(&classic_between(&layer, &with, Some(over_air)).unwrap()).len(),
            inner(&plain).len()
        );
    }

    #[test]
    fn square_gets_evenly_spaced_walls_and_overlapping_infill() {
        let walls = classic(&[block(0.0, 0.0, 20.0, 20.0)], &params(3)).unwrap();
        let described: Vec<_> = walls
            .loops
            .iter()
            .map(|w| (w.depth, w.role, w.is_hole))
            .collect();
        assert_eq!(
            described,
            [
                (0, Role::ExternalPerimeter, false),
                (1, Role::Perimeter, false),
                (2, Role::Perimeter, false)
            ]
        );
        assert!(walls.loops.iter().all(|wall| wall.polygon.is_ccw()));

        // The outer wall's centre line is half a width in; each further
        // wall is one spacing further.
        let spacing = 0.407_08;
        let sides: Vec<_> = walls.loops.iter().map(side).collect();
        for (found, expected) in sides.iter().zip([
            20.0 - 0.45,
            20.0 - 0.45 - 2.0 * spacing,
            20.0 - 0.45 - 4.0 * spacing,
        ]) {
            assert!(
                (found - expected).abs() < 1e-5,
                "side {found} vs {expected}"
            );
        }

        // Infill begins half a spacing inside the last wall, less 15% of
        // the distance between the wall's and the infill's centre lines.
        let inset = spacing / 2.0 - 0.15 * spacing;
        let expected = sides[2] - 2.0 * inset;
        let [infill] = walls.infill.as_slice() else {
            panic!("expected one infill area")
        };
        assert!((infill.area_mm2().sqrt() - expected).abs() < 1e-5);
    }

    #[test]
    fn holes_get_clockwise_walls_that_grow_outward() {
        let mut hole = rect(8.0, 8.0, 12.0, 12.0);
        hole.points.reverse();
        let ring = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![hole],
        };
        let walls = classic(&[ring], &params(2)).unwrap();
        let described: Vec<_> = walls
            .loops
            .iter()
            .map(|w| (w.depth, w.is_hole, w.polygon.is_ccw()))
            .collect();
        assert_eq!(
            described,
            [
                (0, false, true),
                (0, true, false),
                (1, false, true),
                (1, true, false)
            ]
        );
        assert!((side(&walls.loops[1]) - 4.45).abs() < 1e-5);
        assert!((side(&walls.loops[3]) - (4.45 + 2.0 * 0.407_08)).abs() < 1e-5);
        assert_eq!(
            walls
                .infill
                .iter()
                .map(|area| area.holes.len())
                .collect::<Vec<_>>(),
            [1]
        );
    }

    #[test]
    fn narrow_strips_take_only_the_walls_that_fit() {
        // 1 mm across: two walls each side would need about 1.7 mm.
        let strip = classic(&[block(0.0, 0.0, 30.0, 1.0)], &params(3)).unwrap();
        assert_eq!(
            strip
                .loops
                .iter()
                .map(|wall| wall.depth)
                .collect::<Vec<_>>(),
            [0]
        );
        assert!(
            strip.infill.is_empty(),
            "the leftover sliver is a gap, not infill"
        );

        // 2 mm across holds two walls a side and nothing more.
        let strip = classic(&[block(0.0, 0.0, 30.0, 2.0)], &params(5)).unwrap();
        assert_eq!(
            strip
                .loops
                .iter()
                .map(|wall| wall.depth)
                .collect::<Vec<_>>(),
            [0, 1]
        );
    }

    #[test]
    fn short_narrow_islands_get_a_thinner_wall() {
        // 0.6 mm across and short: too tight for a 0.45 mm wall with
        // normal clearance, but not for the slimmer one.
        let dot = classic(&[block(0.0, 0.0, 3.0, 0.6)], &params(2)).unwrap();
        let [wall] = dot.loops.as_slice() else {
            panic!("expected one wall, got {}", dot.loops.len())
        };
        assert_eq!(wall.role, Role::ExternalPerimeter);
        assert!(wall.flow.width < 0.45 && wall.flow.width > 0.40);

        // The same width but long keeps the full-width wall.
        let fin = classic(&[block(0.0, 0.0, 60.0, 0.6)], &params(2)).unwrap();
        assert_eq!(fin.loops[0].flow.width, 0.45);
        // Narrower than any wall: nothing is printed.
        assert!(
            classic(&[block(0.0, 0.0, 3.0, 0.3)], &params(2))
                .unwrap()
                .loops
                .is_empty()
        );
    }

    #[test]
    fn no_walls_leaves_the_whole_region_to_infill() {
        let open = classic(&[block(0.0, 0.0, 10.0, 10.0)], &params(0)).unwrap();
        assert!(open.loops.is_empty());
        assert!((open.infill[0].area_mm2() - 100.0).abs() < 1e-6);
    }

    #[test]
    fn islands_are_walled_one_after_another() {
        let walls = classic(
            &[block(0.0, 0.0, 10.0, 10.0), block(20.0, 0.0, 30.0, 10.0)],
            &params(2),
        )
        .unwrap();
        let min_x: Vec<_> = walls
            .loops
            .iter()
            .map(|w| w.polygon.points.iter().map(|p| p.x).min().unwrap() > scaled(15.0))
            .collect();
        assert_eq!(min_x, [false, false, true, true]);
        assert_eq!(walls.infill.len(), 2);
        assert_eq!(walls.islands, [0..2, 2..4]);
    }

    fn filling(wall_loops: usize) -> Params {
        let base = params(wall_loops);
        Params {
            gap_fill: Some(GapFill {
                flow: base.inner,
                shortest: 0,
            }),
            ..base
        }
    }

    #[test]
    fn the_gap_between_walls_gets_a_line_as_wide_as_it_is() {
        // 1 mm across: a wall each side, 0.45 mm wide with its middle
        // 0.225 mm in, leaves 0.55 mm between the centre lines. The gap
        // is what is further than half the wall spacing from both.
        let strip = classic(&[block(0.0, 0.0, 30.0, 1.0)], &filling(3)).unwrap();
        assert_eq!(strip.loops.len(), 1);
        assert_eq!(strip.gaps_skipped, 0);
        let length: f64 = strip.gap_fill.iter().map(ExtrusionPath::length_mm).sum();
        assert!(length > 28.5 && length < 29.5, "{length} mm of gap fill");
        let expected = 0.55 - 0.407_08;
        for line in &strip.gap_fill {
            assert_eq!(line.role, Role::GapFill);
            assert!(
                line.points
                    .iter()
                    .all(|point| (unscaled(point.y) - 0.5).abs() < 1e-3),
                "the line runs down the middle"
            );
        }
        // Between its ends the line is as wide as the gap.
        let longest = strip
            .gap_fill
            .iter()
            .max_by(|a, b| a.length_mm().total_cmp(&b.length_mm()))
            .unwrap();
        assert!(
            (longest.flow.spacing - expected).abs() < 0.01,
            "{} against {expected}",
            longest.flow.spacing
        );
        assert!(strip.infill.is_empty());
    }

    #[test]
    fn a_part_with_room_for_its_walls_has_no_gap_fill() {
        let walls = classic(&[block(0.0, 0.0, 20.0, 20.0)], &filling(3)).unwrap();
        assert!(walls.gap_fill.is_empty());
        // And the infill is what it is without gap fill.
        let plain = classic(&[block(0.0, 0.0, 20.0, 20.0)], &params(3)).unwrap();
        assert_eq!(walls.infill, plain.infill);
    }

    #[test]
    fn what_a_gap_line_covers_is_not_infilled_too() {
        // 2.2 mm across holds two walls a side and leaves a strip too
        // narrow for a third: a gap line, and no infill under it.
        let strip = classic(&[block(0.0, 0.0, 30.0, 2.2)], &filling(2)).unwrap();
        assert!(!strip.gap_fill.is_empty());
        let infill: f64 = strip.infill.iter().map(ExPolygon::area_mm2).sum();
        assert!(infill < 0.5, "{infill} mm² of infill left");
        let without = classic(&[block(0.0, 0.0, 30.0, 2.2)], &params(2)).unwrap();
        assert!(without.gap_fill.is_empty());
    }

    /// As [`params`], with walls of changing width at the reference's
    /// defaults for a 0.4 mm nozzle.
    fn varying(wall_loops: usize) -> Params {
        Params {
            arachne: Some(ArachneParams {
                min_bead_width: 0.34,
                min_feature_size: 0.1,
                min_length_factor: 0.5,
                wall_transition_length: 0.4,
                wall_transition_angle: 10.0,
                wall_transition_filter_deviation: 0.1,
                wall_distribution_count: 1,
                is_top_or_bottom_layer: false,
                wall_maximum_resolution: 500_000,
                wall_maximum_deviation: 25_000,
            }),
            ..params(wall_loops)
        }
    }

    #[test]
    fn walls_of_changing_width_are_the_classic_ones_where_there_is_room() {
        let square = [block(0.0, 0.0, 20.0, 20.0)];
        let (plain, varied) = (
            classic(&square, &params(2)).unwrap(),
            classic(&square, &varying(2)).unwrap(),
        );
        // The same two loops, as wide and in the same places, and the
        // same area left for infill.
        assert_eq!(varied.loops.len(), 2);
        assert_eq!(varied.islands, plain.islands);
        assert!(varied.gap_fill.is_empty());
        for (ours, theirs) in varied.loops.iter().zip(&plain.loops) {
            assert_eq!(
                (ours.role, ours.depth, ours.is_hole),
                (theirs.role, theirs.depth, false)
            );
            assert!(ours.polygon.is_ccw());
            assert!((ours.flow.width - theirs.flow.width).abs() < 1e-6);
            assert!((side(ours) - side(theirs)).abs() < 1e-3);
        }
        let area = |walls: &Perimeters| walls.infill.iter().map(ExPolygon::area_mm2).sum::<f64>();
        assert!(
            (area(&varied) - area(&plain)).abs() < 0.05,
            "{} against {}",
            area(&varied),
            area(&plain)
        );
    }

    #[test]
    fn a_thin_part_is_one_line_as_wide_as_the_part() {
        // 0.6 mm across: no room for two walls, and classic walls would
        // leave it to gap fill or leave it out.
        let strip = arachne(
            &[block(0.0, 0.0, 30.0, 0.6)],
            &varying(2),
            &varying(2).arachne.unwrap(),
        )
        .unwrap();
        assert!(strip.loops.is_empty() && strip.infill.is_empty());
        assert!(!strip.gap_fill.is_empty());
        let length: f64 = strip.gap_fill.iter().map(ExtrusionPath::length_mm).sum();
        assert!((length - 29.4).abs() < 0.1, "{length} mm");
        for line in &strip.gap_fill {
            assert_eq!(line.role, Role::ExternalPerimeter);
            // The part's width less what the outer wall's line is wider
            // than its spacing, plus what any line is.
            assert!(
                (line.flow.width - 0.6).abs() < 0.01,
                "{} mm wide",
                line.flow.width
            );
            assert!(line.points.iter().all(|p| (p.y - 300_000).abs() < 2_000));
        }
    }

    #[test]
    fn a_hole_is_walled_clockwise_and_nothing_is_laid_twice() {
        let mut ring = block(0.0, 0.0, 20.0, 20.0);
        ring.holes.push(Polygon {
            points: [(8.0, 8.0), (8.0, 12.0), (12.0, 12.0), (12.0, 8.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        });
        let walls = classic(&[ring], &varying(2)).unwrap();
        assert_eq!(walls.loops.len(), 4);
        let holes: Vec<&ExtrusionLoop> = walls.loops.iter().filter(|wall| wall.is_hole).collect();
        assert_eq!(holes.len(), 2);
        assert!(holes.iter().all(|wall| !wall.polygon.is_ccw()));
        assert!(
            walls
                .loops
                .iter()
                .filter(|wall| !wall.is_hole)
                .all(|wall| wall.polygon.is_ccw())
        );
        // Each depth's outer loop before its hole's.
        let order: Vec<(usize, bool)> = walls
            .loops
            .iter()
            .map(|wall| (wall.depth, wall.is_hole))
            .collect();
        assert_eq!(order, [(0, false), (0, true), (1, false), (1, true)]);
    }
}
