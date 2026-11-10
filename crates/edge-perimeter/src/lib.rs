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

pub use settings::WallSettings;

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
    /// its gap.
    pub gap_fill: Vec<ExtrusionPath>,
    /// Gaps left empty because their centre line could not be worked out.
    pub gaps_skipped: usize,
}

/// Concentric constant-width walls for one layer's regions.
pub fn classic(regions: &[ExPolygon], params: &Params) -> Result<Perimeters, Error> {
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
        if inset > 0 {
            let reach = unscaled(inset + params.solid_infill_spacing / 2);
            inset -= scaled(reach * params.infill_overlap_percent / 100.0);
        }
        let min_infill_spacing =
            (params.solid_infill_spacing as f64 * (1.0 - INSET_OVERLAP_TOLERANCE)) as Coord;
        result.infill.extend(edge_clip::offset2(
            &edge_clip::simplify(&last, params.resolution)?,
            -inset - min_infill_spacing / 2,
            min_infill_spacing / 2,
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
            resolution: scaled(0.01),
            gap_fill: None,
        }
    }

    /// Side length in mm of a loop that is an axis-aligned square.
    fn side(wall: &ExtrusionLoop) -> f64 {
        assert_eq!(wall.polygon.points.len(), 4);
        wall.length_mm() / 4.0
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
}
