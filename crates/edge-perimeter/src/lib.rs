//! Walls: the loops that trace each region's outline, and the area left
//! inside them for infill.
//!
//! `classic` is a port of the constant-width generator in the reference
//! (`PerimeterGenerator::process_classic`, PerimeterGenerator.cpp), with
//! the gaps between walls filled by lines of whatever width fits;
//! [`arachne`] lays walls of changing width. Both follow one wall over
//! top surfaces, a precise outer wall, walls over fill that hangs over
//! air (`extra`) and fuzzy skin (`edge-fuzzy`). Not ported: thin-wall
//! detection, spiral vase, and the nesting order loops are printed in,
//! which the writer settles.

mod extra;
mod settings;

use std::ops::Range;

use edge_clip::{ClipError, Join};
use edge_extrusion::{
    ExtrusionLoop, ExtrusionPath, Flow, FlowError, INSET_OVERLAP_TOLERANCE, Role,
};
use edge_geom::{Coord, ExPolygon, scaled, unscaled};

pub use settings::{ArachneSettings, FuzzySettings, WallSettings};

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
    #[error("fuzzy skin: {0}")]
    Fuzzy(#[from] edge_fuzzy::Error),
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
    /// Detail finer than this is dropped from the outline of what is
    /// left for infill. The `resolution` setting itself, where the
    /// walls' own may be finer for arc fitting (PerimeterGenerator.cpp,
    /// "simplify infill contours according to resolution").
    pub infill_resolution: Coord,
    /// How gaps between walls are filled; none leaves them empty.
    pub gap_fill: Option<GapFill>,
    /// Set where a top surface with more of the part above it elsewhere
    /// keeps only its outer wall (`only_one_wall_top`).
    pub one_wall_top: Option<OneWallTop>,
    /// Set where walls are of changing width; the layer is then walled
    /// by [`arachne`].
    pub arachne: Option<ArachneParams>,
    /// The outer wall stands clear of the wall behind it by their widths
    /// and not their spacings, so that the wall behind does not push it
    /// outward (`precise_outer_wall`).
    pub precise_outer_wall: bool,
    /// Set where fill that hangs over air gets walls of its own
    /// (`extra_perimeters_on_overhangs`).
    pub over_air: Option<OverAir>,
    /// Set where walls on this layer get a fuzzy skin (`fuzzy_skin`).
    pub fuzzy: Option<Fuzzy>,
    /// Set on the layers where walls over a steep overhang are laid the
    /// other way round (`overhang_reverse`).
    pub overhang_reverse: Option<OverhangReverse>,
    /// A part too thin for a wall down each side gets one line down its
    /// middle, as wide as the part (`detect_thin_wall`). Classic walls
    /// only: walls of changing width do as much by themselves.
    pub thin_walls: bool,
}

/// What `overhang_reverse` goes by.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverhangReverse {
    /// A wall is over a steep overhang where it lies further than this
    /// outside the layer below, in mm or as a share of its width; at
    /// nothing, every wall is (`overhang_reverse_threshold`).
    pub threshold: edge_config::FloatOrPercent,
    /// The outer wall keeps its way round
    /// (`overhang_reverse_internal_only`).
    pub internal_only: bool,
}

/// What a fuzzy skin needs on one layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fuzzy {
    pub config: edge_fuzzy::FuzzyConfig,
    /// The height the layer was sliced at, in mm: the noise that is not
    /// random is sampled there, so that it runs on from layer to layer.
    pub slice_z: f64,
    /// Set, to the nozzle's bore in mm, where walls that hang over air
    /// are to be left smooth (`fuzzy_skin_skip_overhangs`).
    pub smooth_over_air: Option<f64>,
}

/// What `extra_perimeters_on_overhangs` needs on a layer with one below.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverAir {
    /// What a wall over air is laid with.
    pub flow: Flow,
    /// The layer below counts as holding up this much beyond its
    /// outline: half the nozzle's bore.
    pub grow_lower: Coord,
    /// The number of walls asked for.
    pub wall_loops: usize,
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
    /// Empty on the part's last layer.
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

/// What one layer's fuzzy skin is made within, worked out once for the
/// layer (`group_region_by_fuzzify`, FuzzySkin.cpp). Fuzzy skin here is
/// the same all over the layer: nothing paints it on or off.
struct Fuzzed {
    fuzzy: Fuzzy,
    regions: [edge_fuzzy::Region; 1],
    /// Where walls are near enough the layer below to be roughened.
    supported: Option<Vec<ExPolygon>>,
    /// What holds an outer wall up, and an inner one, where walls over
    /// air are to stay smooth.
    held: Option<[Vec<edge_geom::Polygon>; 2]>,
    layer_height: f64,
    ext_width: Coord,
}

impl Fuzzed {
    fn new(params: &Params, around: Option<&Around<'_>>) -> Result<Option<Self>, Error> {
        let Some(fuzzy) = params.fuzzy else {
            return Ok(None);
        };
        let lower = around.and_then(|around| around.lower);
        let regions = [edge_fuzzy::Region {
            config: fuzzy.config,
            area: Vec::new(),
        }];
        let ext_width = params.outer.scaled_width();
        let supported =
            edge_fuzzy::supported_area(&regions, fuzzy.config.layer_id, lower, ext_width)?;
        let held = match (fuzzy.smooth_over_air, lower) {
            (Some(nozzle), Some(lower)) => {
                let grown = |by: f64| -> Result<Vec<edge_geom::Polygon>, Error> {
                    Ok(edge_clip::offset(lower, scaled(by), Join::MITER)?
                        .into_iter()
                        .flat_map(|piece| std::iter::once(piece.contour).chain(piece.holes))
                        .collect())
                };
                if params.arachne.is_some() {
                    // Half the nozzle beyond the layer below, which is
                    // what these walls are measured against.
                    let all = grown(nozzle / 2.0)?;
                    Some([all.clone(), all])
                } else {
                    // The nearest of the outlines the classic walls are
                    // measured against (`generate_lower_polygons_series`,
                    // with its six samples).
                    let nearest = |width: f64| {
                        let (start, end) = (-0.5 * width, 0.5 * nozzle);
                        grown(start + 0.5 * (end - start) / 5.0)
                    };
                    Some([nearest(params.outer.width)?, nearest(params.inner.width)?])
                }
            }
            _ => None,
        };
        Ok(Some(Self {
            fuzzy,
            regions,
            supported,
            held,
            layer_height: params.outer.height,
            ext_width,
        }))
    }

    fn context(&self) -> edge_fuzzy::Context<'_> {
        edge_fuzzy::Context {
            regions: &self.regions,
            layer_id: self.fuzzy.config.layer_id,
            slice_z: self.fuzzy.slice_z,
            layer_height: self.layer_height,
            supported: self.supported.as_deref(),
            support_external: self.held.as_ref().map(|held| held[0].as_slice()),
            support_internal: self.held.as_ref().map(|held| held[1].as_slice()),
            ext_width: self.ext_width,
        }
    }
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
    /// The same without the overlap: what is inside the walls. The fill
    /// stage goes by it to find what its solid lines leave bare
    /// (`fill_no_overlap`).
    pub infill_no_overlap: Vec<ExPolygon>,
    /// Lines down the middle of the gaps between walls, each as wide as
    /// its gap. With walls of changing width there are no gaps, and what
    /// is here are the walls that are not loops: lines down the middle of
    /// thin parts, in their wall roles.
    pub gap_fill: Vec<ExtrusionPath>,
    /// Gaps left empty because their centre line could not be worked out.
    pub gaps_skipped: usize,
    /// Walls over fill that hangs over air and cannot be bridged, in
    /// the overhang wall's role and in the order they are to be laid:
    /// ahead of the loops of the island they are in.
    pub over_air: Vec<ExtrusionPath>,
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
        return arachne(regions, params, tuning, around);
    }
    let ext_width = params.outer.scaled_width();
    let ext_spacing = params.outer.scaled_spacing();
    let spacing = params.inner.scaled_spacing();
    // Centre-line distance between the external wall and the one behind
    // it: by their widths where the outer wall is to be precise.
    let ext_spacing2 = if params.precise_outer_wall {
        scaled(0.5 * (params.outer.width + params.inner.width))
    } else {
        scaled(0.5 * (params.outer.spacing + params.inner.spacing))
    };
    // Loops keep their full spacing; this smaller figure only decides when
    // an area is too narrow to take another loop at all.
    let min_spacing = (spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
    let ext_min_spacing_smaller =
        (ext_spacing as f64 * (1.0 - SMALLER_EXT_INSET_OVERLAP_TOLERANCE)) as Coord;
    let narrow_flow = params.outer.with_width(
        params.outer.width - 0.5 * SMALLER_EXT_INSET_OVERLAP_TOLERANCE * params.outer.spacing,
    )?;
    let narrow_area = unscaled(ext_width + ext_min_spacing_smaller) * NARROW_LOOP_LENGTH_THRESHOLD;

    let fuzzed = Fuzzed::new(params, around.as_ref())?;
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
        let mut thin: Vec<edge_medial::ThickPolyline> = Vec::new();
        while loop_number >= 0 {
            let mut offsets = Vec::new();
            let mut narrow = Vec::new();
            if depth == 0 && params.thin_walls {
                // The outer wall goes only where there is room for it
                // down both sides: "the minimum thickness of a single
                // loop is ext_width/2 + ext_spacing/2 + spacing/2 +
                // width/2".
                let ext_min_spacing =
                    (ext_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
                offsets = edge_clip::offset2(
                    &last,
                    -((ext_width as f64 / 2.0 + ext_min_spacing as f64 / 2.0 - 1.0) as Coord),
                    (ext_min_spacing as f64 / 2.0 - 1.0) as Coord,
                    Join::MITER,
                )?;
                // What that wall does not cover, less what is too thin
                // to print at all, gets a line down its middle.
                let min_width = scaled(params.outer.nozzle_diameter / 3.0);
                let uncovered = edge_clip::opening(
                    &edge_clip::difference(
                        &last,
                        &edge_clip::offset(&offsets, ext_width / 2 + SAFETY_OFFSET, Join::MITER)?,
                    )?,
                    min_width / 2,
                    Join::MITER,
                )?;
                for piece in &uncovered {
                    match edge_medial::medial_axis(
                        piece,
                        min_width as f64,
                        (ext_width + ext_spacing2) as f64,
                    ) {
                        Ok(found) => thin.extend(found),
                        Err(_) => result.gaps_skipped += 1,
                    }
                }
            } else if depth == 0 {
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

        // The walls are placed smooth and roughened afterwards, so that
        // what lies behind them is where it would be without.
        if let Some(fuzzed) = &fuzzed {
            let context = fuzzed.context();
            for wall in &mut result.loops[first_loop..] {
                wall.polygon = edge_fuzzy::apply_to_polygon(
                    &wall.polygon,
                    &context,
                    wall.depth,
                    !wall.is_hole,
                )?;
            }
        }
        turn_over_overhangs(
            &mut result.loops[first_loop..],
            params,
            around.as_ref(),
            fuzzed.as_ref(),
        )?;
        if result.loops.len() > first_loop {
            result.islands.push(first_loop..result.loops.len());
        }
        // Thin walls are outer walls that are not loops; a fuzzy skin on
        // the outer wall is on them too (`traverse_loops`).
        if let Some(fuzzed) = &fuzzed
            && edge_fuzzy::should_fuzzify(
                &fuzzed.fuzzy.config,
                fuzzed.fuzzy.config.layer_id,
                0,
                true,
            )
        {
            let context = fuzzed.context();
            for line in &mut thin {
                if line.points.len() < 2 || line.widths.len() != 2 * (line.points.len() - 1) {
                    continue;
                }
                // A junction takes the width its segment ends with, the
                // first one the width the first segment starts with.
                let mut wall = edge_arachne::ExtrusionLine::new(0, true);
                wall.junctions = line
                    .points
                    .iter()
                    .enumerate()
                    .map(|(at, &p)| edge_arachne::Junction {
                        p,
                        w: line.widths[if at == 0 { 0 } else { 2 * at - 1 }] as Coord,
                        perimeter_index: 0,
                    })
                    .collect();
                let closed = line.is_closed();
                edge_fuzzy::apply_to_line(&mut wall, &context, true, closed)?;
                line.points = wall.junctions.iter().map(|junction| junction.p).collect();
                line.widths = wall
                    .junctions
                    .windows(2)
                    .flat_map(|pair| [pair[0].w as f64, pair[1].w as f64])
                    .collect();
            }
        }
        result.gap_fill.extend(edge_medial::variable_width(
            &thin,
            Role::ExternalPerimeter,
            &params.outer,
        ));

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
        let (mut top_overlap, mut overlap) = (0, 0);
        if inset > 0 {
            let reach = unscaled(inset + params.solid_infill_spacing / 2);
            if let Some(top) = &params.one_wall_top {
                top_overlap = scaled(reach * top.overlap_percent / 100.0);
            }
            overlap = scaled(reach * params.infill_overlap_percent / 100.0);
            inset -= overlap;
        }
        let min_infill_spacing =
            (params.solid_infill_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
        let not_filled = edge_clip::simplify(&last, params.infill_resolution)?;
        let mut infill = edge_clip::offset2(
            &not_filled,
            -inset - min_infill_spacing / 2,
            min_infill_spacing / 2,
            Join::MITER,
        )?;
        // The same area without the overlap ("get the no-overlap infill
        // expolygons").
        let mut no_overlap = if min_infill_spacing / 2 > overlap {
            edge_clip::offset2(
                &not_filled,
                -inset - min_infill_spacing / 2,
                min_infill_spacing / 2 - overlap,
                Join::MITER,
            )?
        } else {
            edge_clip::offset(&not_filled, -inset - overlap, Join::MITER)?
        };
        // The top fill reaches from its side to the outer wall, half a
        // line nearer than where the inner walls would have started.
        if !top_fills.is_empty() {
            let reached = edge_clip::intersection(
                &fill_clip,
                &edge_clip::offset(&top_fills, ext_spacing / 2, Join::MITER)?,
            )?;
            infill.extend(edge_clip::offset(&reached, top_overlap, Join::MITER)?);
            infill = edge_clip::union_ex(&infill)?;
            no_overlap.extend(reached);
            no_overlap = edge_clip::union_ex(&no_overlap)?;
        }
        result.infill_no_overlap.extend(no_overlap);
        if result.loops.len() > first_loop {
            wall_over_air(
                &mut infill,
                &mut result.over_air,
                &mut result.gaps_skipped,
                params,
                around.as_ref(),
            )?;
        }
        result.infill.extend(infill);
    }
    Ok(result)
}

/// A loop's width may differ by this much along it and still be laid as
/// one width, in scaled units: what the reference allows a path before
/// cutting it (`thick_polyline_to_multi_path`).
const LOOP_WIDTH_TOLERANCE: Coord = 50_000;

/// Walls of changing width for one layer's regions: as many as fit
/// wherever the region is measured, up to `wall_loops`
/// (`PerimeterGenerator::process_arachne`, PerimeterGenerator.cpp).
///
/// Not a port of the whole of it. The reference orders the walls
/// itself; here the loops go to the writer as the classic ones do, to
/// be ordered and given their seams there, a loop whose width changes
/// carrying its width at every point, and the walls that are not loops
/// (lines down the middle of thin parts) are laid with the fill. Where
/// [`Params::one_wall_top`] is set and the layers next to this one are
/// known, a top surface keeps only its outer wall, as the reference has
/// it with `top_surface_expansion` at zero.
pub fn arachne(
    regions: &[ExPolygon],
    params: &Params,
    tuning: &ArachneParams,
    around: Option<Around<'_>>,
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
    // A precise outer wall is made half its overlap further in and set
    // back out by as much, which leaves the overlap to the wall behind.
    let wall_0_inset = if params.precise_outer_wall {
        -(ext_width / 2 - ext_spacing / 2)
    } else {
        0
    };
    let walled =
        |outline: &[ExPolygon], bead_width_0: Coord, inset_count: usize, wall_0_inset: Coord| {
            edge_arachne::generate(
                outline,
                &edge_arachne::Params {
                    bead_width_0,
                    bead_width_x: spacing,
                    inset_count,
                    wall_0_inset,
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
            )
        };
    // One wall over top surfaces needs a wall to give up and the layers
    // next to this one to tell a top surface by.
    let one_wall_top = params
        .one_wall_top
        .as_ref()
        .zip(around.as_ref())
        .filter(|_| params.wall_loops > 1);

    let fuzzed = Fuzzed::new(params, around.as_ref())?;
    let mut result = Perimeters::default();
    for island in regions {
        let first_loop = result.loops.len();
        let simplified = edge_clip::simplify(std::slice::from_ref(island), params.resolution)?;
        // In by what the outer wall is wider than its spacing, so that
        // its outer edge lies on the outline.
        let last = edge_clip::offset(
            &simplified,
            if params.precise_outer_wall {
                -(ext_width - ext_spacing)
            } else {
                -((ext_width as f64 / 2.0 - ext_spacing as f64 / 2.0) as Coord)
            },
            Join::MITER,
        )?;
        let first_pass = walled(
            &last,
            ext_spacing,
            if one_wall_top.is_some() {
                1
            } else {
                params.wall_loops
            },
            wall_0_inset,
        )?;
        let (mut perimeters, mut infill_contour) = (first_pass.toolpaths, first_pass.inner_contour);
        let mut top_fills: Vec<ExPolygon> = Vec::new();

        if let Some((top, around)) = one_wall_top {
            let inner_walls = params.wall_loops - 1;
            let width = params.inner.scaled_width();
            // Too thin an exposed strip is not worth a top surface.
            let min_width = (ext_spacing / 4 + 10).max(top.min_width / 4);
            // What of `contour` has nothing above it and is not over
            // air, opened up to drop what is too thin and grown to hide
            // the wall beside it; and whether anything was uncovered at
            // all (`get_top_expolygons`).
            let tops_of = |contour: &[ExPolygon]| -> Result<(Vec<ExPolygon>, bool), Error> {
                let mut exposed = edge_clip::difference(contour, around.upper)?;
                let uncovered = !exposed.is_empty();
                if !uncovered {
                    return Ok((exposed, false));
                }
                if let Some(lower) = around.lower {
                    let bridges = edge_clip::offset(
                        &edge_clip::difference(&exposed, lower)?,
                        ext_spacing.max(width),
                        Join::MITER,
                    )?;
                    exposed = edge_clip::difference(&exposed, &bridges)?;
                }
                let exposed = edge_clip::offset2(
                    &exposed,
                    -min_width,
                    min_width + (width as f64 * 0.85) as Coord,
                    Join::MITER,
                )?;
                Ok((edge_clip::intersection(&exposed, contour)?, true))
            };
            // The walls there would be with the single wall off, made
            // when first wanted.
            let mut full: Option<edge_arachne::Walls> = None;
            let mut full_pass = || -> Result<edge_arachne::Walls, Error> {
                if full.is_none() {
                    full = Some(walled(&last, ext_spacing, inner_walls + 1, wall_0_inset)?);
                }
                Ok(full.clone().unwrap_or_default())
            };

            // A single wall may be two lines across a part narrower
            // than three, both widened to fill it. Over a top surface
            // that is meant; elsewhere it leaves no room for the inner
            // walls. Where the single pass's walls run away from the
            // full pass's outside the top surface, the full pass's outer
            // walls are taken instead.
            let tolerance = ext_spacing / 10;
            let widest = perimeters
                .iter()
                .flatten()
                .flat_map(|line| &line.junctions)
                .map(|junction| junction.w)
                .max()
                .unwrap_or(0);
            if widest > ext_spacing + 2 * tolerance {
                let nominal = edge_clip::offset(&last, -ext_spacing, Join::MITER)?;
                let (zone, uncovered) = tops_of(&nominal)?;
                if uncovered {
                    let top_zone = edge_clip::offset(&zone, ext_spacing, Join::MITER)?;
                    let full = full_pass()?;
                    if let Some(outer) = full.toolpaths.first().filter(|outer| !outer.is_empty())
                        && off_reference(&perimeters, outer, &top_zone, tolerance)? > width as f64
                    {
                        let footprint =
                            edge_clip::offset(&footprint(outer)?, SAFETY_OFFSET, Join::MITER)?;
                        infill_contour = edge_clip::difference(&nominal, &footprint)?;
                        perimeters = vec![outer.clone()];
                    }
                }
            }

            let (tops, uncovered) = tops_of(&infill_contour)?;
            if uncovered {
                // The inner walls go round what is not a top surface.
                let inner_region = edge_clip::difference(&infill_contour, &tops)?;
                let inner = walled(&inner_region, spacing, inner_walls, 0)?;
                let mut inner_paths = inner.toolpaths;
                if !perimeters.is_empty() {
                    for line in inner_paths.iter_mut().flatten() {
                        line.inset_idx += 1;
                    }
                }
                // The inner walls start at the top surface's edge as
                // well as behind the outer wall, and are numbered from
                // there: numbered again by which wall each touches.
                if perimeters.len() == 1 {
                    let outer_touch = if params.precise_outer_wall {
                        ext_spacing + spacing / 2
                    } else {
                        (ext_spacing + spacing) / 2
                    };
                    renumber_inner_walls(&perimeters[0], &mut inner_paths, outer_touch, spacing);
                }
                perimeters.extend(inner_paths);
                let mut both = tops.clone();
                both.extend(inner.inner_contour);
                infill_contour = edge_clip::union_ex(&both)?;
                top_fills = tops;
            } else {
                // No top surface here: the walls as they are without
                // the single wall.
                let full = full_pass()?;
                perimeters = full.toolpaths;
                infill_contour = full.inner_contour;
            }
        }

        if let Some(fuzzed) = &fuzzed {
            let context = fuzzed.context();
            for wall in perimeters.iter_mut().flatten() {
                if wall.junctions.is_empty() {
                    continue;
                }
                // A wall that is not a loop counts as one round the part.
                let is_contour = !wall.is_closed || wall.is_contour();
                let closed = wall.is_closed;
                edge_fuzzy::apply_to_line(wall, &context, is_contour, closed)?;
            }
        }

        let mut lines = Vec::new();
        for depth in &perimeters {
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
                if wall.is_closed && wall.junctions.len() > 3 {
                    let is_hole = !wall.is_contour();
                    let mut polygon = wall.to_polygon();
                    let mut widths: Vec<Coord> =
                        wall.junctions.iter().map(|junction| junction.w).collect();
                    // A closed wall ends on the point it began at.
                    if polygon.points.len() > 1 && polygon.points.first() == polygon.points.last() {
                        polygon.points.pop();
                        widths.pop();
                    }
                    // Counter-clockwise round material, clockwise round a hole.
                    if polygon.is_ccw() == is_hole {
                        polygon.points.reverse();
                        widths.reverse();
                    }
                    // The width the loop has on average along it.
                    let count = polygon.points.len();
                    let (mut total, mut weighted) = (0.0, 0.0);
                    for at in 0..count {
                        let (a, b) = (polygon.points[at], polygon.points[(at + 1) % count]);
                        let length = ((b.x - a.x) as f64).hypot((b.y - a.y) as f64);
                        total += length;
                        weighted += length * 0.5 * (widths[at] + widths[(at + 1) % count]) as f64;
                    }
                    let average = if total > 0.0 {
                        (weighted / total) as Coord
                    } else {
                        widths[0]
                    };
                    if most - least <= LOOP_WIDTH_TOLERANCE {
                        widths.clear();
                    }
                    let made = ExtrusionLoop {
                        polygon,
                        role,
                        is_hole,
                        depth: wall.inset_idx,
                        flow: flow_of(flow, average)?,
                        widths,
                        clockwise: false,
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
                lines.extend(edge_medial::varying_width(&line, role, flow));
            }
            result.loops.extend(holes);
        }
        turn_over_overhangs(
            &mut result.loops[first_loop..],
            params,
            around.as_ref(),
            fuzzed.as_ref(),
        )?;
        if result.loops.len() > first_loop {
            result.islands.push(first_loop..result.loops.len());
        }
        result.gap_fill.extend(lines);

        // Infill reaches from the inside of the innermost wall back
        // into it by the overlap set.
        let wall_count = perimeters.len();
        let inside_spacing = if wall_count == 1 {
            ext_spacing2
        } else {
            spacing
        };
        if edge_clip::offset(&infill_contour, -inside_spacing / 2, Join::MITER)?.is_empty() {
            infill_contour.clear();
        }
        let reach = match wall_count {
            0 => 0,
            1 => ext_spacing,
            _ => spacing,
        };
        let inset = scaled(unscaled(reach) * params.infill_overlap_percent / 100.0);
        let min_infill_spacing =
            (params.solid_infill_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
        let not_filled = edge_clip::simplify(&infill_contour, params.infill_resolution)?;
        let mut infill = edge_clip::offset2(
            &not_filled,
            -min_infill_spacing / 2,
            inset + min_infill_spacing / 2,
            Join::MITER,
        )?;
        // The same area without the overlap.
        let mut no_overlap = edge_clip::offset2(
            &not_filled,
            -min_infill_spacing / 2,
            min_infill_spacing / 2,
            Join::MITER,
        )?;
        // The top fill overlaps the wall beside it by the amount set for
        // top surfaces.
        if let (Some((top, _)), false) = (one_wall_top, top_fills.is_empty()) {
            let top_inset = scaled(unscaled(reach) * top.overlap_percent / 100.0);
            infill.extend(edge_clip::offset(&top_fills, top_inset, Join::MITER)?);
            infill = edge_clip::union_ex(&infill)?;
            no_overlap.extend(top_fills.iter().cloned());
            no_overlap = edge_clip::union_ex(&no_overlap)?;
        }
        result.infill_no_overlap.extend(no_overlap);
        if result.loops.len() > first_loop {
            wall_over_air(
                &mut infill,
                &mut result.over_air,
                &mut result.gaps_skipped,
                params,
                around.as_ref(),
            )?;
        }
        result.infill.extend(infill);
    }
    Ok(result)
}

/// Marks an island's walls to be laid clockwise where
/// [`Params::overhang_reverse`] is set and one of them is over a steep
/// overhang: the walls round the part if any of those is, and the walls
/// round holes if any of those is (`detect_steep_overhang` and
/// `reorient_perimeters`, PerimeterGenerator.cpp). With a fuzzy skin they
/// always are, as in the reference.
fn turn_over_overhangs(
    loops: &mut [ExtrusionLoop],
    params: &Params,
    around: Option<&Around<'_>>,
    fuzzed: Option<&Fuzzed>,
) -> Result<(), Error> {
    let Some((reverse, lower)) = params
        .overhang_reverse
        .zip(around.and_then(|around| around.lower))
    else {
        return Ok(());
    };
    // Round the part, and round holes.
    let mut steep = [false, false];
    if let Some(fuzzed) = fuzzed {
        let config = &fuzzed.fuzzy.config;
        // Only where the walls round the part are fuzzy; and round
        // holes, where those are fuzzy as well.
        let fuzzy = |is_contour| edge_fuzzy::should_fuzzify(config, config.layer_id, 0, is_contour);
        let has = |is_hole: bool| loops.iter().any(|wall| wall.is_hole == is_hole);
        steep = [
            fuzzy(true) && has(false),
            fuzzy(true) && fuzzy(false) && has(true),
        ];
    }
    for wall in loops.iter() {
        let found = &mut steep[usize::from(wall.is_hole)];
        if *found {
            continue;
        }
        let width = wall.flow.width;
        let threshold = reverse.threshold.resolve(width);
        // "Special case: reverse on every even (from GUI POV) layer".
        if threshold < 1e-4 {
            *found = true;
            continue;
        }
        let limit = edge_clip::offset(lower, scaled(threshold - 0.5 * width), Join::MITER)?;
        let mut outline = wall.polygon.points.clone();
        outline.extend(outline.first().copied());
        *found = !edge_clip::lines_outside(&[outline], &limit)?.is_empty();
    }
    for wall in loops.iter_mut() {
        let outer = wall.role == Role::ExternalPerimeter;
        if steep[usize::from(wall.is_hole)] && !(reverse.internal_only && outer) {
            wall.clockwise = true;
        }
    }
    Ok(())
}

/// Where [`Params::over_air`] is set and there is a layer below, walls
/// what of an island's fill hangs over air and cannot be bridged, and
/// takes what those walls cover out of the fill
/// (`PerimeterGenerator::apply_extra_perimeters`).
fn wall_over_air(
    infill: &mut Vec<ExPolygon>,
    walls: &mut Vec<ExtrusionPath>,
    gaps_skipped: &mut usize,
    params: &Params,
    around: Option<&Around<'_>>,
) -> Result<(), Error> {
    let Some((over_air, lower)) = params
        .over_air
        .as_ref()
        .zip(around.and_then(|around| around.lower))
    else {
        return Ok(());
    };
    // What holds a line up: the layer below and half a nozzle beyond it.
    let lower = edge_clip::offset(lower, over_air.grow_lower, Join::MITER)?;
    let (extra, filled) = extra::over_overhangs_counting(
        infill,
        &lower,
        over_air.wall_loops,
        &over_air.flow,
        gaps_skipped,
    )?;
    if extra.is_empty() {
        return Ok(());
    }
    walls.extend(extra.into_iter().flatten());
    *infill = edge_clip::difference(infill, &filled)?;
    Ok(())
}

/// Whether two walls run alongside each other: some point of one within
/// half their two widths of the other, or within `min_threshold` if that
/// is more (`arachne_lines_touch`).
fn lines_touch(
    a: &edge_arachne::ExtrusionLine,
    b: &edge_arachne::ExtrusionLine,
    min_threshold: f64,
) -> bool {
    let one_way = |from: &edge_arachne::ExtrusionLine, to: &edge_arachne::ExtrusionLine| {
        from.junctions.iter().any(|junction| {
            let (px, py) = (junction.p.x as f64, junction.p.y as f64);
            to.junctions.windows(2).any(|pair| {
                let (sx, sy) = (pair[0].p.x as f64, pair[0].p.y as f64);
                let (dx, dy) = (pair[1].p.x as f64 - sx, pair[1].p.y as f64 - sy);
                let length2 = dx * dx + dy * dy;
                let along = if length2 > 0.0 {
                    (((px - sx) * dx + (py - sy) * dy) / length2).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // The width of `to` at its nearest point.
                let width = pair[0].w as f64 + along * (pair[1].w - pair[0].w) as f64;
                let touching = min_threshold.max(0.5 * (junction.w as f64 + width));
                (sx + along * dx - px).hypot(sy + along * dy - py) <= touching
            })
        })
    };
    one_way(a, b) || one_way(b, a)
}

/// Numbers the inner walls made in a second pass by how far each is
/// from the outer walls: one touching an outer wall is 1, one touching
/// a wall numbered k that was itself made as k + 1 is k + 1, and any
/// other takes one more than the deepest wall it touches. A wall that
/// touches none keeps the number it was made with
/// (`renumber_inner_walls_by_depth`). The walls come back grouped by
/// number, group g holding those numbered g + 1.
fn renumber_inner_walls(
    outer_walls: &[edge_arachne::ExtrusionLine],
    inner_walls: &mut Vec<edge_arachne::VariableWidthLines>,
    outer_touch: Coord,
    inner_touch: Coord,
) {
    let mut lines: Vec<edge_arachne::ExtrusionLine> = std::mem::take(inner_walls)
        .into_iter()
        .flatten()
        .filter(|line| !line.junctions.is_empty())
        .collect();
    if lines.is_empty() {
        return;
    }
    // Walls whose bounds, grown by their widest point, do not overlap
    // cannot touch.
    let bounds_of = |line: &edge_arachne::ExtrusionLine| -> [Coord; 4] {
        let mut bounds = [Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN];
        let mut widest = 0;
        for junction in &line.junctions {
            bounds[0] = bounds[0].min(junction.p.x);
            bounds[1] = bounds[1].min(junction.p.y);
            bounds[2] = bounds[2].max(junction.p.x);
            bounds[3] = bounds[3].max(junction.p.y);
            widest = widest.max(junction.w);
        }
        [
            bounds[0] - widest,
            bounds[1] - widest,
            bounds[2] + widest,
            bounds[3] + widest,
        ]
    };
    let overlap = |a: &[Coord; 4], b: &[Coord; 4]| {
        a[0] <= b[2] && b[0] <= a[2] && a[1] <= b[3] && b[1] <= a[3]
    };
    let bounds: Vec<[Coord; 4]> = lines.iter().map(bounds_of).collect();
    // Whole units put walls made exactly the touching distance apart a
    // few of them further.
    let rounding = 1_000.0;
    let (outer_threshold, inner_threshold) =
        (outer_touch as f64 + rounding, inner_touch as f64 + rounding);
    let touch = |a: usize, b: usize| {
        overlap(&bounds[a], &bounds[b]) && lines_touch(&lines[a], &lines[b], inner_threshold)
    };

    // The walls numbered from the outer walls.
    let mut depth = vec![0usize; lines.len()];
    let mut frontier = Vec::new();
    for outer in outer_walls {
        let outer_bounds = bounds_of(outer);
        for at in 0..lines.len() {
            if depth[at] == 0
                && lines[at].inset_idx == 1
                && overlap(&outer_bounds, &bounds[at])
                && lines_touch(outer, &lines[at], outer_threshold)
            {
                depth[at] = 1;
                frontier.push(at);
            }
        }
    }
    let mut numbered = frontier.clone();
    let mut level = 1;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for at in 0..lines.len() {
            if depth[at] == 0
                && lines[at].inset_idx == level + 1
                && frontier.iter().any(|&from| touch(from, at))
            {
                depth[at] = level + 1;
                next.push(at);
            }
        }
        numbered.extend(&next);
        frontier = next;
        level += 1;
    }
    // The others, working away from those.
    frontier = numbered;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for at in 0..lines.len() {
            if depth[at] != 0 {
                continue;
            }
            let deepest = frontier
                .iter()
                .filter(|&&from| touch(from, at))
                .map(|&from| depth[from])
                .max()
                .unwrap_or(0);
            if deepest > 0 {
                depth[at] = deepest + 1;
                next.push(at);
            }
        }
        frontier = next;
    }

    let numbers: Vec<usize> = lines
        .iter()
        .zip(&depth)
        .map(|(line, &depth)| if depth > 0 { depth } else { line.inset_idx })
        .collect();
    for (mut line, number) in lines.drain(..).zip(numbers) {
        line.inset_idx = number;
        if inner_walls.len() < number {
            inner_walls.resize(number, Vec::new());
        }
        inner_walls[number - 1].push(line);
    }
}

/// The points of each wall with a length to it.
fn centre_lines(walls: &[edge_arachne::ExtrusionLine]) -> Vec<Vec<edge_geom::Point2>> {
    walls
        .iter()
        .filter(|line| line.junctions.len() >= 2)
        .map(|line| line.junctions.iter().map(|junction| junction.p).collect())
        .collect()
}

/// How much of `walls`, in scaled units of length, runs further than
/// `tolerance` from the walls of `reference`, outside `excluded`
/// (`length_off_reference`).
fn off_reference(
    walls: &[edge_arachne::VariableWidthLines],
    reference: &[edge_arachne::ExtrusionLine],
    excluded: &[ExPolygon],
    tolerance: Coord,
) -> Result<f64, Error> {
    let own: Vec<Vec<edge_geom::Point2>> =
        walls.iter().flat_map(|depth| centre_lines(depth)).collect();
    let theirs = centre_lines(reference);
    let near = edge_clip::thicken(theirs.iter().map(Vec::as_slice), tolerance)?;
    let off = edge_clip::lines_outside(&edge_clip::lines_outside(&own, &near)?, excluded)?;
    Ok(off
        .iter()
        .flat_map(|line| line.windows(2))
        .map(|pair| ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64))
        .sum())
}

/// The ground `walls` cover, each piece at the wider of its two ends
/// (`walls_footprint`).
fn footprint(walls: &[edge_arachne::ExtrusionLine]) -> Result<Vec<ExPolygon>, Error> {
    let mut covered = Vec::new();
    for pair in walls.iter().flat_map(|line| line.junctions.windows(2)) {
        let width = pair[0].w.max(pair[1].w);
        if width > 0 {
            covered.extend(edge_clip::thicken(
                [[pair[0].p, pair[1].p].as_slice()],
                width / 2,
            )?);
        }
    }
    Ok(edge_clip::union_ex(&covered)?)
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
            widths: Vec::new(),
            clockwise: false,
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
            infill_resolution: scaled(0.01),
            gap_fill: None,
            arachne: None,
            precise_outer_wall: false,
            over_air: None,
            fuzzy: None,
            overhang_reverse: None,
            thin_walls: false,
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
            None,
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
    fn walls_of_changing_width_thin_to_one_over_a_top_surface_too() {
        // As for the classic walls: a 40 x 20 mm layer with more of the
        // part on its left half and nothing on its right.
        let layer = [block(0.0, 0.0, 40.0, 20.0)];
        let (upper, lower) = ([block(0.0, 0.0, 20.0, 20.0)], [block(0.0, 0.0, 40.0, 20.0)]);
        let with = Params {
            one_wall_top: Some(OneWallTop {
                min_width: scaled(0.9),
                infill_width: scaled(0.45),
                bridge_margin: scaled(1.0),
                wall_loops: 3,
                overlap_percent: 25.0,
            }),
            ..varying(3)
        };
        let between = Around {
            upper: &upper,
            lower: Some(&lower),
        };
        let thinned = classic_between(&layer, &with, Some(between)).unwrap();
        let plain = classic(&layer, &varying(3)).unwrap();
        let lengths = |walls: &Perimeters, outer: bool| -> f64 {
            walls
                .loops
                .iter()
                .filter(|wall| (wall.depth == 0) == outer)
                .map(ExtrusionLoop::length_mm)
                .sum()
        };
        // One outer wall round the whole layer either way.
        assert!((lengths(&thinned, true) - lengths(&plain, true)).abs() < 1e-3);
        // The inner walls go round the covered half only, a little way
        // on under the top fill: well under what they were all round.
        assert_eq!(
            thinned.loops.iter().filter(|wall| wall.depth > 0).count(),
            2
        );
        assert!(lengths(&thinned, false) < 0.75 * lengths(&plain, false));
        let furthest = thinned
            .loops
            .iter()
            .filter(|wall| wall.depth > 0)
            .flat_map(|wall| &wall.polygon.points)
            .map(|point| point.x)
            .max()
            .unwrap();
        assert!(
            (scaled(19.0)..scaled(23.0)).contains(&furthest),
            "{furthest}"
        );
        // And what they gave up is filled.
        let area = |walls: &Perimeters| walls.infill.iter().map(ExPolygon::area_mm2).sum::<f64>();
        assert!(area(&thinned) > area(&plain) + 10.0);
        // Without the layers either side it cannot be told, and the
        // walls are all there; the same with one wall asked for.
        assert_eq!(classic(&layer, &with).unwrap().loops, plain.loops);
        let single = Params {
            wall_loops: 1,
            ..with
        };
        assert_eq!(
            classic_between(&layer, &single, Some(between))
                .unwrap()
                .loops
                .len(),
            1
        );
        // A layer wholly covered keeps every wall.
        let covered = Around {
            upper: &layer,
            lower: Some(&lower),
        };
        assert_eq!(
            classic_between(&layer, &with, Some(covered)).unwrap().loops,
            plain.loops
        );
    }

    #[test]
    fn a_loop_whose_width_changes_stays_a_loop_and_carries_its_widths() {
        // A frame 1.4 mm thick on one side and 2 mm on the other three:
        // too thin there for four full lines.
        let mut frame = block(0.0, 0.0, 20.0, 20.0);
        frame.holes.push(Polygon {
            points: [(1.4, 2.0), (1.4, 18.0), (18.0, 18.0), (18.0, 2.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        });
        let walls = classic(&[frame], &varying(2)).unwrap();
        let changing: Vec<&ExtrusionLoop> = walls
            .loops
            .iter()
            .filter(|wall| !wall.widths.is_empty())
            .collect();
        assert!(!changing.is_empty());
        for wall in &changing {
            assert_eq!(wall.widths.len(), wall.polygon.points.len());
            assert_eq!(wall.polygon.is_ccw(), !wall.is_hole);
            let (least, most) = (
                *wall.widths.iter().min().unwrap(),
                *wall.widths.iter().max().unwrap(),
            );
            assert!(most - least > LOOP_WIDTH_TOLERANCE);
            // Laid at a width between its narrowest and its widest.
            let own = scaled(wall.flow.width - 0.2 * (1.0 - std::f64::consts::FRAC_PI_4));
            assert!((least..=most).contains(&own), "{least} {own} {most}");
        }
        // The outer walls are one width all round, inside and out.
        assert!(
            walls
                .loops
                .iter()
                .filter(|wall| wall.depth == 0)
                .all(|wall| wall.widths.is_empty())
        );
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

    /// The width in mm of an axis-aligned loop round a block at the origin.
    fn across(wall: &ExtrusionLoop) -> f64 {
        let xs = wall.polygon.points.iter().map(|point| point.x);
        unscaled(xs.clone().max().unwrap() - xs.min().unwrap())
    }

    #[test]
    fn a_precise_outer_wall_stays_put_and_the_wall_behind_gives_way() {
        // Walls 0.45 mm wide lie 0.407 mm apart: the inner wall overlaps
        // the outer by what their rounded edges leave. Made precise, the
        // outer wall is where it was and the inner one further in.
        for base in [params(2), varying(2)] {
            let precise = Params {
                precise_outer_wall: true,
                ..base
            };
            let plain = classic(&[block(0.0, 0.0, 20.0, 20.0)], &base).unwrap();
            let walls = classic(&[block(0.0, 0.0, 20.0, 20.0)], &precise).unwrap();
            assert_eq!((plain.loops.len(), walls.loops.len()), (2, 2));
            assert!((across(&walls.loops[0]) - across(&plain.loops[0])).abs() < 1e-3);
            assert!((across(&walls.loops[0]) - 19.55).abs() < 1e-3);
            let (width, spacing) = (base.outer.width, base.outer.spacing);
            let apart =
                |walls: &Perimeters| (across(&walls.loops[0]) - across(&walls.loops[1])) / 2.0;
            assert!((apart(&plain) - spacing).abs() < 2e-3);
            // Walls of changing width give up half of what the classic
            // ones do, as in the reference.
            let expected = if base.arachne.is_some() {
                0.5 * (width + spacing)
            } else {
                width
            };
            assert!((apart(&walls) - expected).abs() < 2e-3, "{}", apart(&walls));
        }
    }

    /// A fuzzy skin of this kind on the fourth layer, otherwise as the
    /// reference has it unset: 0.2 mm deep, a point every 0.3 mm.
    fn fuzzy(kind: edge_fuzzy::FuzzyType) -> Fuzzy {
        Fuzzy {
            config: FuzzySettings {
                kind,
                ..FuzzySettings::default()
            }
            .config(3),
            slice_z: 0.7,
            smooth_over_air: None,
        }
    }

    #[test]
    fn a_fuzzy_skin_roughens_the_walls_asked_for_and_nothing_else() {
        let part = [block(0.0, 0.0, 20.0, 20.0)];
        for base in [params(2), varying(2)] {
            let rough = Params {
                fuzzy: Some(fuzzy(edge_fuzzy::FuzzyType::External)),
                ..base
            };
            let plain = classic(&part, &base).unwrap();
            let walls = classic(&part, &rough).unwrap();
            assert_eq!(walls.loops.len(), 2);
            // The outer wall is a point every 0.3 mm or so, each up to
            // 0.2 mm to one side, and the longer for it.
            let outer = &walls.loops[0];
            assert!(
                outer.polygon.points.len() > 200,
                "{}",
                outer.polygon.points.len()
            );
            assert!(outer.length_mm() > plain.loops[0].length_mm() * 1.05);
            assert!((across(outer) - across(&plain.loops[0])).abs() <= 0.4 + 1e-6);
            assert!(outer.polygon.is_ccw());
            // The wall behind it and the fill are where they were.
            assert_eq!(walls.loops[1].polygon, plain.loops[1].polygon);
            assert_eq!(walls.infill, plain.infill);
            // And the same again another time.
            assert_eq!(classic(&part, &rough).unwrap(), walls);

            // Every wall, where every wall is asked for.
            let all = Params {
                fuzzy: Some(fuzzy(edge_fuzzy::FuzzyType::AllWalls)),
                ..base
            };
            let walls = classic(&part, &all).unwrap();
            assert!(
                walls
                    .loops
                    .iter()
                    .all(|wall| wall.polygon.points.len() > 200)
            );
        }
    }

    #[test]
    fn a_fuzzy_skin_keeps_off_walls_with_nothing_near_beneath() {
        // The layer below is the left half only.
        let part = [block(0.0, 0.0, 20.0, 20.0)];
        let lower = [block(0.0, 0.0, 10.0, 20.0)];
        let around = Some(Around {
            upper: &[],
            lower: Some(&lower),
        });
        for base in [params(2), varying(2)] {
            let rough = Params {
                fuzzy: Some(fuzzy(edge_fuzzy::FuzzyType::External)),
                ..base
            };
            let smooth = classic(&part, &base).unwrap().loops[0].polygon.clone();
            let walls = classic_between(&part, &rough, around).unwrap();
            let outer = &walls.loops[0].polygon;
            let xs = smooth.points.iter().map(|point| point.x);
            let (left, right) = (xs.clone().min().unwrap(), xs.max().unwrap());
            let ys = smooth.points.iter().map(|point| point.y);
            let (low, high) = (ys.clone().min().unwrap(), ys.max().unwrap());
            let on_smooth = |point: &Point2| {
                point.x == left || point.x == right || point.y == low || point.y == high
            };
            // Roughened over the layer below, and left alone from a
            // line's width and the fuzz's depth beyond it.
            let (near, far): (Vec<&Point2>, Vec<&Point2>) = outer
                .points
                .iter()
                .partition(|point| point.x < scaled(10.0));
            assert!(near.iter().filter(|point| !on_smooth(point)).count() > 50);
            assert!(
                far.iter()
                    .filter(|point| point.x > scaled(11.0))
                    .all(|point| on_smooth(point))
            );
        }
    }

    #[test]
    fn a_part_too_thin_for_two_walls_gets_one_line_down_its_middle() {
        // Half a millimetre across: no room for a wall down each side.
        let fin = [block(0.0, 0.0, 10.0, 0.5)];
        let thin = Params {
            thin_walls: true,
            ..params(2)
        };
        let walls = classic(&fin, &thin).unwrap();
        assert!(walls.loops.is_empty() && walls.islands.is_empty());
        assert_eq!(walls.gap_fill.len(), 1);
        let line = &walls.gap_fill[0];
        // An outer wall down its middle and most of its length, spaced
        // to fill the fin: wider than it by what a line's rounded edges
        // leave empty, as the reference lays it.
        assert_eq!(line.role, Role::ExternalPerimeter);
        let wide = 0.5 + 0.2 * (1.0 - std::f64::consts::FRAC_PI_4);
        assert!((line.flow.width - wide).abs() < 0.02, "{}", line.flow.width);
        assert!(
            line.points
                .iter()
                .all(|point| (point.y - scaled(0.25)).abs() < 2_000)
        );
        assert!(
            line.length_mm() > 9.0 && line.length_mm() < 10.01,
            "{}",
            line.length_mm()
        );
        assert!(walls.infill.is_empty());

        // A part with room for its walls is walled as it is without,
        // though a loop may begin at another of its corners.
        let part = [block(0.0, 0.0, 20.0, 20.0)];
        let (with, without) = (
            classic(&part, &thin).unwrap(),
            classic(&part, &params(2)).unwrap(),
        );
        assert_eq!(with.infill, without.infill);
        assert!(with.gap_fill.is_empty());
        let sides = |walls: &Perimeters| walls.loops.iter().map(side).collect::<Vec<f64>>();
        assert_eq!(sides(&with), sides(&without));

        // A fuzzy skin on the outer wall is on the thin wall too.
        let rough = Params {
            fuzzy: Some(fuzzy(edge_fuzzy::FuzzyType::External)),
            ..thin
        };
        let fuzzed = classic(&fin, &rough).unwrap();
        let points: usize = fuzzed.gap_fill.iter().map(|line| line.points.len()).sum();
        assert!(points > 20, "{points}");
    }

    #[test]
    fn inner_walls_are_numbered_by_how_far_they_are_from_the_outer_wall() {
        // Lines 0.4 mm wide and 0.4 mm apart, the outer wall along y = 0.
        let line = |y: f64, inset_idx: usize| edge_arachne::ExtrusionLine {
            inset_idx,
            is_odd: false,
            is_closed: false,
            junctions: [0.0, 10.0]
                .map(|x| edge_arachne::Junction {
                    p: Point2::from_mm(x, y),
                    w: scaled(0.4),
                    perimeter_index: inset_idx,
                })
                .to_vec(),
        };
        let outer = [line(0.0, 0)];
        // Made behind the outer wall as 1 and 2; and from the far edge
        // (a top surface's) as 1, with one behind that as 2; and one off
        // by itself.
        let mut inner = vec![
            vec![line(0.4, 1), line(1.6, 1), line(9.0, 1)],
            vec![line(0.8, 2), line(1.2, 2)],
        ];
        renumber_inner_walls(&outer, &mut inner, scaled(0.4), scaled(0.4));
        let found: Vec<Vec<i64>> = inner
            .iter()
            .map(|group| {
                let mut ys: Vec<i64> = group.iter().map(|wall| wall.junctions[0].p.y).collect();
                ys.sort();
                ys
            })
            .collect();
        // 0.4 touches the outer wall; 0.8 was made as 2 and touches it;
        // 1.2 touches 0.8 and is 3, 1.6 touches 1.2 and is 4; the one
        // that touches nothing keeps its 1.
        assert_eq!(
            found,
            [
                vec![scaled(0.4), scaled(9.0)],
                vec![scaled(0.8)],
                vec![scaled(1.2)],
                vec![scaled(1.6)]
            ]
        );
        assert!(
            inner
                .iter()
                .enumerate()
                .all(|(group, walls)| { walls.iter().all(|wall| wall.inset_idx == group + 1) })
        );
    }

    #[test]
    fn walls_over_a_steep_overhang_are_marked_to_turn_about() {
        // The layer below reaches to x = 18 of this layer's 20: its
        // right-hand walls hang 2 mm over air.
        let part = [block(0.0, 0.0, 20.0, 20.0)];
        let lower = [block(0.0, 0.0, 18.0, 20.0)];
        let around = |lower: &'static [ExPolygon]| {
            Some(Around {
                upper: &[],
                lower: Some(lower),
            })
        };
        let lower: &'static [ExPolygon] = Box::leak(Box::new(lower));
        let whole: &'static [ExPolygon] = Box::leak(Box::new([block(0.0, 0.0, 20.0, 20.0)]));
        let reverse = |threshold, internal_only| OverhangReverse {
            threshold,
            internal_only,
        };
        let half = edge_config::FloatOrPercent::Percent(50.0);
        for base in [params(2), varying(2)] {
            let turned = |reverse: OverhangReverse, lower: &'static [ExPolygon]| -> Vec<bool> {
                let asked = Params {
                    overhang_reverse: Some(reverse),
                    ..base
                };
                classic_between(&part, &asked, around(lower))
                    .unwrap()
                    .loops
                    .iter()
                    .map(|wall| wall.clockwise)
                    .collect()
            };
            // Both walls of the island, once one of them is over it.
            assert_eq!(turned(reverse(half, false), lower), [true, true]);
            // The outer wall can be left as it is.
            assert_eq!(turned(reverse(half, true), lower), [false, true]);
            // Nothing over air, nothing turned; unless every wall is to be.
            assert_eq!(turned(reverse(half, false), whole), [false, false]);
            let always = edge_config::FloatOrPercent::Value(0.0);
            assert_eq!(turned(reverse(always, false), whole), [true, true]);
            // Not asked, not marked.
            let plain = classic_between(&part, &base, around(lower)).unwrap();
            assert!(plain.loops.iter().all(|wall| !wall.clockwise));
        }
    }
}
