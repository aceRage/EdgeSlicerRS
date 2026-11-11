//! Fuzzy skin on the walls of a layer whose regions have settings of
//! their own: each wall is roughened part by part, by the settings of
//! the region each part lies in.
//!
//! A port of both `apply_fuzzy_skin` overloads of the reference's
//! Feature/FuzzySkin/FuzzySkin.cpp and what they lean on there.

use edge_arachne::{ExtrusionLine, Junction};
use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

use crate::Error;
use crate::config::{FuzzyConfig, FuzzyType, should_fuzzify};
use crate::fuzz::{fuzzy_extrusion_line, fuzzy_polyline};
use crate::split::{SplitLineJunction, split_line};

/// Fuzzy skin settings and where on the layer they hold: one entry of
/// the reference's `regions_by_fuzzify` (PerimeterGenerator.hpp), which
/// its `group_region_by_fuzzify` fills with one entry for each distinct
/// set of settings among a layer's regions.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub config: FuzzyConfig,
    /// Where the settings hold. Empty means everywhere, which is what the
    /// reference gives a layer's only entry. With several entries each is
    /// its regions' slices grown by the reference's `ClipperSafetyOffset`
    /// (10 units), and where two overlap the later one wins.
    pub area: Vec<ExPolygon>,
}

/// What a layer's walls are roughened by: the part of the reference's
/// `PerimeterGenerator` that `apply_fuzzy_skin` reads.
#[derive(Clone, Copy, Debug)]
pub struct Context<'a> {
    /// `regions_by_fuzzify`. A single entry is taken to hold everywhere,
    /// whatever its area says, unless `supported` is set: the reference's
    /// short cut for the usual case.
    pub regions: &'a [Region],
    /// The layer's index, from 0.
    pub layer_id: i32,
    /// `slice_z`: the height the layer is sliced at, in mm. The coherent
    /// noises are sampled at it.
    pub slice_z: f64,
    /// The layer's height in mm.
    pub layer_height: f64,
    /// `fuzzy_supported_area`: with it, only what lies inside is
    /// roughened at all; a wall bridging beyond it stays smooth.
    /// [`supported_area`] works it out.
    pub supported: Option<&'a [ExPolygon]>,
    /// What the reference's `fuzzy_support_region` returns for the outer
    /// wall: the region the layer below carries, for
    /// `fuzzy_skin_skip_overhangs`. `None` when that is not wanted on
    /// this layer; the caller has applied the reference's
    /// `fuzzy_skip_overhangs_wanted` ([`skip_overhangs_wanted`] is the
    /// settings' part of it). For walls of changing width the reference
    /// uses the same set for every wall, `lower_slices_polygons()`.
    pub support_external: Option<&'a [Polygon]>,
    /// The same for the inner walls.
    pub support_internal: Option<&'a [Polygon]>,
    /// `ext_perimeter_flow.scaled_width()`: the outer wall's width.
    pub ext_width: Coord,
}

impl Context<'_> {
    /// FuzzySkin.cpp, `fuzzy_support_region`, the choice between the two
    /// series.
    fn support(&self, is_external: bool) -> Option<&[Polygon]> {
        if is_external {
            self.support_external
        } else {
            self.support_internal
        }
    }
}

/// The settings' part of the reference's `fuzzy_skip_overhangs_wanted`
/// (FuzzySkin.cpp): whether any region that can be fuzzy asks to skip
/// overhangs. The reference also wants a layer below, overhang detection
/// on (`detect_overhang_wall`) and a layer above the raft
/// (`layer_id > raft_layers`); those are the caller's to check.
pub fn skip_overhangs_wanted(regions: &[Region]) -> bool {
    regions.iter().any(|region| {
        region.config.kind != FuzzyType::None
            && region.config.kind != FuzzyType::Disabled
            && region.config.skip_overhangs
    })
}

/// The area fuzzy skin is kept to on a layer with a layer below it: the
/// `fuzzy_supported_area` that the reference's `group_region_by_fuzzify`
/// (FuzzySkin.cpp) works out. Walls farther than a line width plus the
/// noise's reach from the layer below are bridging and stay smooth.
/// `None` without a layer below, or when no region is fuzzy here.
pub fn supported_area(
    regions: &[Region],
    layer_id: i32,
    lower_slices: Option<&[ExPolygon]>,
    ext_width: Coord,
) -> Result<Option<Vec<ExPolygon>>, Error> {
    let Some(lower_slices) = lower_slices else {
        return Ok(None);
    };
    let mut max_thickness = None;
    for region in regions {
        if should_fuzzify(&region.config, layer_id, 0, true)
            || should_fuzzify(&region.config, layer_id, 0, false)
        {
            max_thickness = Some(region.config.thickness.max(max_thickness.unwrap_or(0)));
        }
    }
    let Some(max_thickness) = max_thickness else {
        return Ok(None);
    };
    let grown = edge_clip::offset(lower_slices, ext_width + max_thickness, Join::MITER)?;
    Ok(Some(grown))
}

/// FuzzySkin.cpp, `MergedFuzzyRegion`.
struct MergedRegion<'a> {
    config: &'a FuzzyConfig,
    /// Empty means everywhere, until `restrict_to_supported` has run.
    area: Vec<ExPolygon>,
}

/// FuzzySkin.cpp, `same_fuzzy_effect`: whether two sets of settings
/// roughen a wall the same way. The kind and the first-layer switch only
/// decide which walls are roughened.
fn same_fuzzy_effect(a: &FuzzyConfig, b: &FuzzyConfig) -> bool {
    a.thickness == b.thickness
        && a.point_distance == b.point_distance
        && a.noise == b.noise
        && a.scale == b.scale
        && a.octaves == b.octaves
        && a.persistence == b.persistence
        && a.mode == b.mode
        && a.ripples_per_layer == b.ripples_per_layer
        && a.ripple_offset == b.ripple_offset
        && a.layers_between_ripple_offset == b.layers_between_ripple_offset
        && a.skip_overhangs == b.skip_overhangs
}

/// FuzzySkin.cpp, `collect_merged_fuzzy_regions`: the regions that
/// roughen this wall, those with the same effect joined into one. Where
/// the object's own setting and a painted area both apply with the same
/// noise, the wall is then not cut at the edge of the paint.
fn collect_merged_fuzzy_regions(
    regions: &[Region],
    layer_id: i32,
    loop_idx: usize,
    is_contour: bool,
) -> Result<Vec<MergedRegion<'_>>, Error> {
    let mut merged_regions: Vec<MergedRegion> = Vec::with_capacity(regions.len());
    for region in regions {
        if !should_fuzzify(&region.config, layer_id, loop_idx, is_contour) {
            continue;
        }

        let same = merged_regions
            .iter_mut()
            .find(|merged| same_fuzzy_effect(merged.config, &region.config));
        match same {
            Some(merged) => {
                if merged.area.is_empty() {
                    // Already everywhere, nothing to add.
                } else if region.area.is_empty() {
                    merged.area.clear();
                } else {
                    merged.area.extend_from_slice(&region.area);
                }
            }
            None => merged_regions.push(MergedRegion {
                config: &region.config,
                area: region.area.clone(),
            }),
        }
    }

    for merged in &mut merged_regions {
        if !merged.area.is_empty() {
            merged.area = edge_clip::union_ex(&merged.area)?;
        }
    }

    Ok(merged_regions)
}

/// The step both `apply_fuzzy_skin` overloads (FuzzySkin.cpp) take
/// before splitting: each region loses what a later one covers, so that
/// no stretch of wall is roughened twice. Later regions win, as later
/// modifiers do.
fn make_exclusive(merged_regions: &mut [MergedRegion]) -> Result<(), Error> {
    for i in 0..merged_regions.len() {
        for j in i + 1..merged_regions.len() {
            if !merged_regions[i].area.is_empty() && !merged_regions[j].area.is_empty() {
                merged_regions[i].area =
                    edge_clip::difference(&merged_regions[i].area, &merged_regions[j].area)?;
            }
        }
    }
    Ok(())
}

/// FuzzySkin.cpp, `restrict_to_supported`. Afterwards an empty area
/// means nothing to roughen, no longer everywhere.
fn restrict_to_supported(
    merged_regions: &mut [MergedRegion],
    supported: Option<&[ExPolygon]>,
) -> Result<(), Error> {
    let Some(supported) = supported else {
        return Ok(());
    };
    for merged in merged_regions {
        merged.area = if merged.area.is_empty() {
            supported.to_vec()
        } else {
            edge_clip::intersection(&merged.area, supported)?
        };
    }
    Ok(())
}

/// Roughens the walls' fixed-width outline `polygon`, each part by the
/// settings of the region it lies in: the reference's
/// `apply_fuzzy_skin(const Polygon&, ...)` (FuzzySkin.cpp). `loop_idx`
/// counts walls from the outside in, from 0; `is_contour` is false for
/// the wall of a hole. A wall no region roughens comes back as it was.
///
/// Where the wall has to be cut, each part inside a region is roughened
/// as an open run by [`fuzzy_polyline`] and put back between its two
/// ends, which stay in place; the regions take their turns in order, each
/// on what the ones before it left.
///
/// Different from the reference on purpose: it cannot fail, and this
/// returns an error when the clipping of the regions does.
pub fn apply_to_polygon(
    polygon: &Polygon,
    ctx: &Context,
    loop_idx: usize,
    is_contour: bool,
) -> Result<Polygon, Error> {
    let slice_z = ctx.slice_z;
    let regions = ctx.regions;
    let support = ctx.support(loop_idx == 0);
    let supported = ctx.supported;
    if regions.len() == 1 && supported.is_none() {
        // Optimization.
        let config = &regions[0].config;
        if !should_fuzzify(config, ctx.layer_id, loop_idx, is_contour) {
            return Ok(polygon.clone());
        }

        let mut fuzzified = polygon.clone();
        fuzzy_polyline(&mut fuzzified.points, true, slice_z, config, support);
        return Ok(fuzzified);
    }

    let mut merged_regions =
        collect_merged_fuzzy_regions(regions, ctx.layer_id, loop_idx, is_contour)?;
    if merged_regions.is_empty() {
        return Ok(polygon.clone());
    }

    // Fast path: a single merged region that holds everywhere is applied
    // without splitting.
    if merged_regions.len() == 1 && merged_regions[0].area.is_empty() && supported.is_none() {
        let mut fuzzified = polygon.clone();
        fuzzy_polyline(
            &mut fuzzified.points,
            true,
            slice_z,
            merged_regions[0].config,
            support,
        );
        return Ok(fuzzified);
    }

    make_exclusive(&mut merged_regions)?;
    restrict_to_supported(&mut merged_regions, supported)?;

    // Split the loop into lines with different settings, and fuzz them
    // separately.
    let mut fuzzified = polygon.clone();
    for r in &merged_regions {
        let mut splitted = split_line(&fuzzified.points, &r.area, true);
        if splitted.is_empty() {
            // No intersection, skip.
            continue;
        }

        let Some(first_non_clipped) = splitted.iter().position(|junction| !junction.clipped) else {
            // The entire polygon is fuzzified.
            fuzzy_polyline(&mut fuzzified.points, true, slice_z, r.config, support);
            continue;
        };
        // Start from a junction that is not clipped, so that a clipped
        // run wrapping round the end needs no joining across the seam.
        splitted.rotate_left(first_non_clipped);

        let mut segment: Vec<Point2> = Vec::with_capacity(splitted.len());
        let mut points: Vec<Point2> = Vec::with_capacity(splitted.len());
        let fuzzy_current_segment = |segment: &mut Vec<Point2>, points: &mut Vec<Point2>| {
            points.push(segment[0]);
            let back = segment[segment.len() - 1];
            fuzzy_polyline(segment, false, slice_z, r.config, support);
            points.append(segment);
            points.push(back);
        };

        for p in &splitted {
            if p.clipped {
                segment.push(p.p);
            } else if segment.is_empty() {
                points.push(p.p);
            } else {
                segment.push(p.p);
                fuzzy_current_segment(&mut segment, &mut points);
            }
        }
        if !segment.is_empty() {
            // Close the loop.
            segment.push(splitted[0].p);
            fuzzy_current_segment(&mut segment, &mut points);
        }
        fuzzified.points = points;
    }

    Ok(fuzzified)
}

/// Roughens a wall of changing width in place, each part by the settings
/// of the region it lies in: the reference's
/// `apply_fuzzy_skin(Arachne::ExtrusionLine*, ...)` (FuzzySkin.cpp). The
/// wall's `inset_idx` says which wall it is; `is_contour` is false for
/// the wall of a hole. `closed` is false for a thin wall that collapsed
/// into a single open line: it runs down the middle of the part, which
/// painted areas only just reach, so the regions are grown by a tenth of
/// the outer wall's width to cover it.
///
/// Where the wall has to be cut, a new junction takes the width of the
/// junction its segment starts from, and each part inside a region is
/// roughened by [`fuzzy_extrusion_line`] and put back between its two
/// ends.
///
/// Different from the reference on purpose: it cannot fail, and this
/// returns an error when the clipping of the regions does.
pub fn apply_to_line(
    line: &mut ExtrusionLine,
    ctx: &Context,
    is_contour: bool,
    closed: bool,
) -> Result<(), Error> {
    let slice_z = ctx.slice_z;
    let layer_height = ctx.layer_height;
    let regions = ctx.regions;
    let support = ctx.support(line.inset_idx == 0);
    let supported = ctx.supported;
    if regions.len() == 1 && supported.is_none() {
        // Optimization.
        let config = &regions[0].config;
        if should_fuzzify(config, ctx.layer_id, line.inset_idx, is_contour) {
            fuzzy_extrusion_line(&mut line.junctions, slice_z, layer_height, config, support);
        }
        return Ok(());
    }

    let mut merged_regions =
        collect_merged_fuzzy_regions(regions, ctx.layer_id, line.inset_idx, is_contour)?;
    if merged_regions.is_empty() {
        return Ok(());
    }

    // Fast path: a single merged region that holds everywhere is applied
    // without splitting.
    if merged_regions.len() == 1 && merged_regions[0].area.is_empty() && supported.is_none() {
        fuzzy_extrusion_line(
            &mut line.junctions,
            slice_z,
            layer_height,
            merged_regions[0].config,
            support,
        );
        return Ok(());
    }

    if !closed {
        for r in &mut merged_regions {
            // Growing nothing gives nothing: everywhere stays everywhere.
            if !r.area.is_empty() {
                r.area = edge_clip::offset(&r.area, ctx.ext_width / 10, Join::MITER)?;
            }
        }
    }

    make_exclusive(&mut merged_regions)?;
    restrict_to_supported(&mut merged_regions, supported)?;

    // Split the line into pieces with different settings, and fuzz them
    // separately.
    for r in &merged_regions {
        let path: Vec<Point2> = line.junctions.iter().map(|junction| junction.p).collect();
        // Always as an open path, as the reference passes it.
        let splitted = split_line(&path, &r.area, false);
        if splitted.is_empty() {
            // No intersection, skip.
            continue;
        }

        if splitted.iter().all(|junction| junction.clipped) {
            // The entire line is fuzzified. (An open path's last junction
            // is never clipped, so this is the reference's branch kept
            // for what it is: a line wholly inside goes the other way,
            // as one piece between its own two ends.)
            fuzzy_extrusion_line(
                &mut line.junctions,
                slice_z,
                layer_height,
                r.config,
                support,
            );
            continue;
        }

        let current_ext = std::mem::take(&mut line.junctions);
        let mut segment: Vec<Junction> = Vec::with_capacity(current_ext.len());
        let out = &mut line.junctions;

        let fuzzy_current_segment = |segment: &mut Vec<Junction>, out: &mut Vec<Junction>| {
            out.push(segment[0]);
            let back = segment[segment.len() - 1];
            fuzzy_extrusion_line(segment, slice_z, layer_height, r.config, support);
            out.append(segment);
            out.push(back);
        };

        let to_ex_junction = |j: &SplitLineJunction| -> Junction {
            let mut res = current_ext[j.get_src_index()];
            if !j.is_src() {
                res.p = j.p;
            }
            res
        };

        for p in &splitted {
            if p.clipped {
                segment.push(to_ex_junction(p));
            } else if segment.is_empty() {
                out.push(to_ex_junction(p));
            } else {
                segment.push(to_ex_junction(p));
                fuzzy_current_segment(&mut segment, out);
            }
        }
        if !segment.is_empty() {
            fuzzy_current_segment(&mut segment, out);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FuzzyMode, NoiseType};
    use crate::testing::{mm, outline_distance, rect, rect_region};
    use edge_geom::scaled;

    fn config(kind: FuzzyType) -> FuzzyConfig {
        FuzzyConfig {
            kind,
            ..FuzzyConfig::default()
        }
    }

    fn everywhere(config: FuzzyConfig) -> Region {
        Region {
            config,
            area: Vec::new(),
        }
    }

    fn context(regions: &[Region]) -> Context<'_> {
        Context {
            regions,
            layer_id: 3,
            slice_z: 0.8,
            layer_height: 0.2,
            supported: None,
            support_external: None,
            support_internal: None,
            ext_width: scaled(0.45),
        }
    }

    fn square() -> Polygon {
        rect(0.0, 0.0, 20.0, 20.0)
    }

    fn direct(polygon: &Polygon, config: &FuzzyConfig) -> Polygon {
        let mut fuzzified = polygon.clone();
        fuzzy_polyline(&mut fuzzified.points, true, 0.8, config, None);
        fuzzified
    }

    fn junction(x: f64, y: f64) -> Junction {
        Junction {
            p: mm(x, y),
            w: 450_000,
            perimeter_index: 0,
        }
    }

    /// The square as a closed outer wall of changing width.
    fn square_line() -> ExtrusionLine {
        ExtrusionLine {
            inset_idx: 0,
            is_odd: false,
            is_closed: true,
            junctions: vec![
                junction(0.0, 0.0),
                junction(20.0, 0.0),
                junction(20.0, 20.0),
                junction(0.0, 20.0),
                junction(0.0, 0.0),
            ],
        }
    }

    fn direct_line(line: &ExtrusionLine, config: &FuzzyConfig) -> ExtrusionLine {
        let mut fuzzified = line.clone();
        fuzzy_extrusion_line(&mut fuzzified.junctions, 0.8, 0.2, config, None);
        fuzzified
    }

    #[test]
    fn one_region_everywhere_is_a_plain_fuzz() {
        let regions = [everywhere(config(FuzzyType::External))];
        let ctx = context(&regions);
        let out = apply_to_polygon(&square(), &ctx, 0, true).unwrap();
        assert_ne!(out, square());
        assert_eq!(out, direct(&square(), &regions[0].config));

        // Walls the kind leaves out come back as they went in.
        assert_eq!(
            apply_to_polygon(&square(), &ctx, 1, true).unwrap(),
            square()
        );
        assert_eq!(
            apply_to_polygon(&square(), &ctx, 0, false).unwrap(),
            square()
        );
        // And so does everything on the first layer.
        let first_layer = Context { layer_id: 0, ..ctx };
        assert_eq!(
            apply_to_polygon(&square(), &first_layer, 0, true).unwrap(),
            square()
        );
        // No regions at all, or none that is fuzzy.
        assert_eq!(
            apply_to_polygon(&square(), &context(&[]), 0, true).unwrap(),
            square()
        );
        let plain = [
            everywhere(config(FuzzyType::None)),
            everywhere(config(FuzzyType::Disabled)),
        ];
        assert_eq!(
            apply_to_polygon(&square(), &context(&plain), 0, true).unwrap(),
            square()
        );

        // The reference's short cut: an only region holds everywhere,
        // whatever area it names.
        let odd = [Region {
            config: config(FuzzyType::External),
            area: vec![rect_region(50.0, 50.0, 60.0, 60.0)],
        }];
        assert_eq!(
            apply_to_polygon(&square(), &context(&odd), 0, true).unwrap(),
            direct(&square(), &odd[0].config)
        );
    }

    #[test]
    fn the_wall_number_picks_the_overhang_region() {
        let regions = [everywhere(FuzzyConfig {
            skip_overhangs: true,
            ..config(FuzzyType::AllWalls)
        })];
        // The outer wall is carried on the left, the inner ones on the
        // right.
        let left = [rect(-5.0, -5.0, 10.0, 25.0)];
        let right = [rect(10.0, -5.0, 25.0, 25.0)];
        let ctx = Context {
            support_external: Some(&left),
            support_internal: Some(&right),
            ..context(&regions)
        };
        let on_outline = |p: &Point2| outline_distance(*p, &square().points) <= 1.0;
        let outer = apply_to_polygon(&square(), &ctx, 0, true).unwrap();
        assert!(
            outer
                .points
                .iter()
                .filter(|p| p.x > scaled(11.0))
                .all(on_outline)
        );
        assert!(
            !outer
                .points
                .iter()
                .filter(|p| p.x < scaled(9.0))
                .all(on_outline)
        );
        let inner = apply_to_polygon(&square(), &ctx, 1, true).unwrap();
        assert!(
            inner
                .points
                .iter()
                .filter(|p| p.x < scaled(9.0))
                .all(on_outline)
        );
        assert!(
            !inner
                .points
                .iter()
                .filter(|p| p.x > scaled(11.0))
                .all(on_outline)
        );

        let mut line = square_line();
        apply_to_line(&mut line, &ctx, true, true).unwrap();
        assert!(
            line.junctions
                .iter()
                .map(|j| &j.p)
                .filter(|p| p.x > scaled(11.0))
                .all(on_outline)
        );
        let mut line = ExtrusionLine {
            inset_idx: 2,
            ..square_line()
        };
        apply_to_line(&mut line, &ctx, true, true).unwrap();
        assert!(
            line.junctions
                .iter()
                .map(|j| &j.p)
                .filter(|p| p.x < scaled(9.0))
                .all(on_outline)
        );
    }

    #[test]
    fn supported_half_leaves_the_other_half_untouched() {
        let cfg = config(FuzzyType::External);
        let regions = [everywhere(cfg)];
        let supported = [rect_region(-5.0, -5.0, 10.0, 25.0)];
        let ctx = Context {
            supported: Some(&supported),
            ..context(&regions)
        };
        let out = apply_to_polygon(&square(), &ctx, 0, true).unwrap().points;
        let outline = square().points;
        let limit = cfg.thickness + 2;

        // Outside the supported area there is nothing but the square's own
        // two corners; the wall is cut where it enters and leaves it, and
        // those two new points stay on the outline, around the fuzzy run.
        let (cut_in, cut_out) = (mm(10.0, 20.0), mm(10.0, 0.0));
        let outside: Vec<Point2> = out
            .iter()
            .copied()
            .filter(|p| p.x > scaled(10.0) + limit)
            .collect();
        assert_eq!(outside, [mm(20.0, 0.0), mm(20.0, 20.0)]);
        // The walk starts at the first junction that is not clipped: the
        // cut on the bottom side. The run inside then goes from the top
        // cut round through the seam and back to that first point, which
        // the reference adds again to close the loop.
        assert_eq!(out[0], cut_out);
        assert_eq!(out[1], mm(20.0, 0.0));
        assert_eq!(out[2], mm(20.0, 20.0));
        assert_eq!(out[3], cut_in);
        assert_eq!(out[out.len() - 1], cut_out);
        // In between, the roughened run: the open fuzz of the piece
        // inside, between its two ends.
        let mut run = vec![cut_in, mm(0.0, 20.0), mm(0.0, 0.0), cut_out];
        fuzzy_polyline(&mut run, false, 0.8, &cfg, None);
        assert_eq!(&out[4..out.len() - 1], &run[..]);
        assert!(run.len() > 100);
        let mut furthest: f64 = 0.0;
        for &p in &run {
            let distance = outline_distance(p, &outline);
            assert!(distance <= limit as f64);
            assert!(p.x <= scaled(10.0) + limit);
            furthest = furthest.max(distance);
        }
        assert!(furthest > 0.8 * cfg.thickness as f64);

        // Supported all round, it is the plain fuzz; supported nowhere,
        // the wall is left alone.
        let all = [rect_region(-5.0, -5.0, 25.0, 25.0)];
        let ctx_all = Context {
            supported: Some(&all),
            ..ctx
        };
        assert_eq!(
            apply_to_polygon(&square(), &ctx_all, 0, true).unwrap(),
            direct(&square(), &cfg)
        );
        let nowhere = [rect_region(50.0, 50.0, 60.0, 60.0)];
        let ctx_none = Context {
            supported: Some(&nowhere),
            ..ctx
        };
        assert_eq!(
            apply_to_polygon(&square(), &ctx_none, 0, true).unwrap(),
            square()
        );
        let ctx_empty = Context {
            supported: Some(&[]),
            ..ctx
        };
        assert_eq!(
            apply_to_polygon(&square(), &ctx_empty, 0, true).unwrap(),
            square()
        );
    }

    #[test]
    fn two_regions_each_roughen_their_own_part() {
        let thin = FuzzyConfig {
            thickness: scaled(0.05),
            ..config(FuzzyType::External)
        };
        let thick = FuzzyConfig {
            thickness: scaled(0.4),
            ..config(FuzzyType::External)
        };
        let regions = [
            Region {
                config: thin,
                area: vec![rect_region(-5.0, -5.0, 10.0, 25.0)],
            },
            Region {
                config: thick,
                area: vec![rect_region(10.0, -5.0, 25.0, 25.0)],
            },
        ];
        let out = apply_to_polygon(&square(), &context(&regions), 0, true)
            .unwrap()
            .points;
        let outline = square().points;
        let margin = scaled(0.5);
        let (mut left, mut right) = (Vec::new(), Vec::new());
        for &p in &out {
            let distance = outline_distance(p, &outline);
            if p.x < scaled(10.0) - margin {
                left.push(distance);
            } else if p.x > scaled(10.0) + margin {
                right.push(distance);
            }
        }
        assert!(
            left.len() > 80 && right.len() > 80,
            "{} {}",
            left.len(),
            right.len()
        );
        let furthest = |distances: &[f64]| distances.iter().copied().fold(0.0, f64::max);
        // The left keeps to its 0.05 mm and the right uses its 0.4 mm.
        assert!(furthest(&left) <= thin.thickness as f64 + 2.0);
        assert!(furthest(&left) > 0.8 * thin.thickness as f64);
        assert!(furthest(&right) <= thick.thickness as f64 + 2.0);
        assert!(furthest(&right) > 0.8 * thick.thickness as f64);

        // Where two regions overlap the later one wins: the thick one
        // spread over everything leaves the thin one nothing.
        let overlapping = [
            regions[0].clone(),
            Region {
                config: thick,
                area: vec![rect_region(-5.0, -5.0, 25.0, 25.0)],
            },
        ];
        let out = apply_to_polygon(&square(), &context(&overlapping), 0, true).unwrap();
        assert_eq!(out, direct(&square(), &thick));
        // The other way round the thin one takes its half back.
        let reversed = [overlapping[1].clone(), overlapping[0].clone()];
        let out = apply_to_polygon(&square(), &context(&reversed), 0, true)
            .unwrap()
            .points;
        let left: Vec<f64> = out
            .iter()
            .filter(|p| p.x < scaled(10.0) - margin)
            .map(|&p| outline_distance(p, &outline))
            .collect();
        assert!(left.len() > 80);
        assert!(furthest(&left) <= thin.thickness as f64 + 2.0);

        // A region that is not for this wall takes no part: only the
        // left half is roughened on a hole's wall when the right is for
        // contours only.
        let mixed = [
            Region {
                config: FuzzyConfig {
                    kind: FuzzyType::All,
                    ..thin
                },
                area: regions[0].area.clone(),
            },
            regions[1].clone(),
        ];
        let out = apply_to_polygon(&square(), &context(&mixed), 0, false)
            .unwrap()
            .points;
        let right: Vec<Point2> = out
            .iter()
            .copied()
            .filter(|p| p.x > scaled(10.0) + margin)
            .collect();
        assert_eq!(right, [mm(20.0, 0.0), mm(20.0, 20.0)]);
        assert!(out.len() > 100);
    }

    #[test]
    fn regions_with_the_same_effect_are_merged() {
        // The object's own setting on the left and a painted area on the
        // right, with the same noise: one region, and no cut between them.
        let external = config(FuzzyType::External);
        let painted = config(FuzzyType::All);
        assert!(same_fuzzy_effect(&external, &painted));
        let regions = [
            Region {
                config: external,
                area: vec![rect_region(-5.0, -5.0, 10.0, 25.0)],
            },
            Region {
                config: painted,
                area: vec![rect_region(10.0, -5.0, 25.0, 25.0)],
            },
        ];
        let merged = collect_merged_fuzzy_regions(&regions, 3, 0, true).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].config, &external);
        assert_eq!(merged[0].area.len(), 1);
        assert!((merged[0].area[0].area_mm2() - 900.0).abs() < 1e-6);
        let out = apply_to_polygon(&square(), &context(&regions), 0, true).unwrap();
        assert_eq!(out, direct(&square(), &external));

        // On a hole's wall only the painted one applies, and the wall is
        // cut at its edge.
        let merged = collect_merged_fuzzy_regions(&regions, 3, 0, false).unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].config, &painted);
        let hole = apply_to_polygon(&square(), &context(&regions), 0, false).unwrap();
        assert_ne!(hole, out);
        assert!(hole.points.contains(&mm(0.0, 0.0)) && hole.points.contains(&mm(0.0, 20.0)));

        // Merged with a region that holds everywhere, the whole is
        // everywhere, in either order.
        for order in [[0, 1], [1, 0]] {
            let pair = [
                everywhere(external),
                Region {
                    config: painted,
                    area: vec![rect_region(10.0, -5.0, 25.0, 25.0)],
                },
            ];
            let pair = [pair[order[0]].clone(), pair[order[1]].clone()];
            let merged = collect_merged_fuzzy_regions(&pair, 3, 0, true).unwrap();
            assert_eq!(merged.len(), 1);
            assert!(merged[0].area.is_empty());
            let out = apply_to_polygon(&square(), &context(&pair), 0, true).unwrap();
            assert_eq!(out, direct(&square(), &external));
        }

        // Anything that changes the fuzz keeps regions apart.
        let base = config(FuzzyType::External);
        let different = [
            FuzzyConfig {
                thickness: 1,
                ..base
            },
            FuzzyConfig {
                point_distance: 1,
                ..base
            },
            FuzzyConfig {
                noise: NoiseType::Perlin,
                ..base
            },
            FuzzyConfig { scale: 2.0, ..base },
            FuzzyConfig { octaves: 5, ..base },
            FuzzyConfig {
                persistence: 0.4,
                ..base
            },
            FuzzyConfig {
                mode: FuzzyMode::Combined,
                ..base
            },
            FuzzyConfig {
                ripples_per_layer: 3,
                ..base
            },
            FuzzyConfig {
                ripple_offset: 10.0,
                ..base
            },
            FuzzyConfig {
                layers_between_ripple_offset: 2,
                ..base
            },
            FuzzyConfig {
                skip_overhangs: true,
                ..base
            },
        ];
        for other in different {
            assert!(!same_fuzzy_effect(&base, &other), "{other:?}");
        }
        // Which walls, and the first layer, do not.
        assert!(same_fuzzy_effect(
            &base,
            &FuzzyConfig {
                kind: FuzzyType::Hole,
                ..base
            }
        ));
        assert!(same_fuzzy_effect(
            &base,
            &FuzzyConfig {
                first_layer: true,
                ..base
            }
        ));
    }

    #[test]
    fn line_with_one_region_everywhere_is_a_plain_fuzz() {
        let regions = [everywhere(config(FuzzyType::External))];
        let ctx = context(&regions);
        let mut line = square_line();
        apply_to_line(&mut line, &ctx, true, true).unwrap();
        assert_ne!(line, square_line());
        assert_eq!(line, direct_line(&square_line(), &regions[0].config));

        // An inner wall, a hole's wall and the first layer are left out.
        let mut inner = ExtrusionLine {
            inset_idx: 1,
            ..square_line()
        };
        let before = inner.clone();
        apply_to_line(&mut inner, &ctx, true, true).unwrap();
        assert_eq!(inner, before);
        let mut hole = square_line();
        apply_to_line(&mut hole, &ctx, false, true).unwrap();
        assert_eq!(hole, square_line());
        let mut first = square_line();
        apply_to_line(&mut first, &Context { layer_id: 0, ..ctx }, true, true).unwrap();
        assert_eq!(first, square_line());

        // Two regions of one effect, one of them everywhere: still plain.
        let pair = [
            Region {
                config: config(FuzzyType::All),
                area: vec![rect_region(10.0, -5.0, 25.0, 25.0)],
            },
            everywhere(config(FuzzyType::External)),
        ];
        let mut line = square_line();
        apply_to_line(&mut line, &context(&pair), true, true).unwrap();
        assert_eq!(line, direct_line(&square_line(), &pair[0].config));
    }

    #[test]
    fn line_in_a_supported_half() {
        let cfg = FuzzyConfig {
            mode: FuzzyMode::Extrusion,
            ..config(FuzzyType::External)
        };
        let regions = [everywhere(cfg)];
        // Carried on the right half.
        let supported = [rect_region(10.0, -5.0, 25.0, 25.0)];
        let ctx = Context {
            supported: Some(&supported),
            ..context(&regions)
        };
        // Widths that tell the junctions apart.
        let mut line = square_line();
        for (i, junction) in line.junctions.iter_mut().enumerate() {
            junction.w = 400_000 + 10_000 * i as Coord;
        }
        let source = line.clone();
        apply_to_line(&mut line, &ctx, true, true).unwrap();
        let out = &line.junctions;

        // The line starts outside: its first junction, then the new one
        // where it goes in, with the width of the junction before it.
        assert_eq!(out[0], source.junctions[0]);
        let cut_in = Junction {
            p: mm(10.0, 0.0),
            ..source.junctions[0]
        };
        assert_eq!(out[1], cut_in);
        // The run inside is roughened as a line of its own, between its
        // two ends; the one where it comes out has the width of the
        // junction its segment starts from.
        let cut_out = Junction {
            p: mm(10.0, 20.0),
            ..source.junctions[2]
        };
        let mut run = vec![cut_in, source.junctions[1], source.junctions[2], cut_out];
        fuzzy_extrusion_line(&mut run, 0.8, 0.2, &cfg, None);
        assert!(run.len() > 100);
        assert_eq!(&out[2..2 + run.len()], &run[..]);
        // Then the end of the run and the rest of the line, untouched.
        assert_eq!(
            &out[2 + run.len()..],
            &[cut_out, source.junctions[3], source.junctions[4]]
        );
        // Only widths changed, and only inside.
        let outline = square().points;
        assert!(out.iter().all(|j| outline_distance(j.p, &outline) <= 1.0));
        assert!(
            out.iter()
                .filter(|j| j.p.x < scaled(10.0))
                .all(|j| source.junctions.contains(j))
        );
        assert!(run.iter().any(|j| j.w > 470_000) && run.iter().any(|j| j.w < 420_000));

        // A line wholly inside is one run between its own two ends.
        let all = [rect_region(-5.0, -5.0, 25.0, 25.0)];
        let ctx_all = Context {
            supported: Some(&all),
            ..ctx
        };
        let mut inside = source.clone();
        apply_to_line(&mut inside, &ctx_all, true, true).unwrap();
        let mut whole = source.junctions.clone();
        fuzzy_extrusion_line(&mut whole, 0.8, 0.2, &cfg, None);
        assert_eq!(inside.junctions[0], source.junctions[0]);
        assert_eq!(&inside.junctions[1..inside.junctions.len() - 1], &whole[..]);
        assert_eq!(
            inside.junctions[inside.junctions.len() - 1],
            source.junctions[4]
        );

        // And one wholly outside is left alone.
        let nowhere = [rect_region(50.0, 50.0, 60.0, 60.0)];
        let ctx_none = Context {
            supported: Some(&nowhere),
            ..ctx
        };
        let mut outside = source.clone();
        apply_to_line(&mut outside, &ctx_none, true, true).unwrap();
        assert_eq!(outside, source);
    }

    #[test]
    fn open_thin_wall_reaches_a_region_grown_by_a_tenth_of_a_width() {
        let near = FuzzyConfig {
            thickness: scaled(0.1),
            ..config(FuzzyType::External)
        };
        let far = config(FuzzyType::External);
        let regions = [
            // Ends at y = 1; the line below runs 0.02 mm beyond it.
            Region {
                config: near,
                area: vec![rect_region(0.0, 0.0, 20.0, 1.0)],
            },
            Region {
                config: far,
                area: vec![rect_region(50.0, 50.0, 60.0, 60.0)],
            },
        ];
        let ctx = context(&regions);
        let thin_wall = ExtrusionLine {
            inset_idx: 0,
            is_odd: true,
            is_closed: false,
            junctions: vec![junction(2.0, 1.02), junction(18.0, 1.02)],
        };
        // As a closed wall's piece it misses the region.
        let mut line = thin_wall.clone();
        apply_to_line(&mut line, &ctx, true, true).unwrap();
        assert_eq!(line, thin_wall);
        // As an open one the region comes 0.045 mm out to meet it.
        let mut line = thin_wall.clone();
        apply_to_line(&mut line, &ctx, true, false).unwrap();
        assert!(line.junctions.len() > 40);
        assert_eq!(line.junctions[0], thin_wall.junctions[0]);
        assert_eq!(
            line.junctions[line.junctions.len() - 1],
            thin_wall.junctions[1]
        );
        let furthest = line
            .junctions
            .iter()
            .map(|j| (j.p.y - scaled(1.02)).abs())
            .max()
            .unwrap();
        assert!(furthest > scaled(0.08) && furthest <= near.thickness + 1);
        // Further off than that it is missed either way.
        let mut beyond = ExtrusionLine {
            junctions: vec![junction(2.0, 1.1), junction(18.0, 1.1)],
            ..thin_wall.clone()
        };
        let before = beyond.clone();
        apply_to_line(&mut beyond, &ctx, true, false).unwrap();
        assert_eq!(beyond, before);
    }

    #[test]
    fn two_regions_on_a_line() {
        let narrow = FuzzyConfig {
            thickness: scaled(0.05),
            ..config(FuzzyType::External)
        };
        let wide = FuzzyConfig {
            thickness: scaled(0.4),
            ..config(FuzzyType::External)
        };
        let regions = [
            Region {
                config: narrow,
                area: vec![rect_region(-5.0, -5.0, 10.0, 25.0)],
            },
            Region {
                config: wide,
                area: vec![rect_region(10.0, -5.0, 25.0, 25.0)],
            },
        ];
        let mut line = square_line();
        apply_to_line(&mut line, &context(&regions), true, true).unwrap();
        let outline = square().points;
        let margin = scaled(0.5);
        let furthest = |keep: &dyn Fn(&Point2) -> bool| {
            line.junctions
                .iter()
                .filter(|j| keep(&j.p))
                .map(|j| outline_distance(j.p, &outline))
                .fold(0.0, f64::max)
        };
        let left = furthest(&|p| p.x < scaled(10.0) - margin);
        let right = furthest(&|p| p.x > scaled(10.0) + margin);
        assert!(left <= narrow.thickness as f64 + 2.0 && left > 0.8 * narrow.thickness as f64);
        assert!(right <= wide.thickness as f64 + 2.0 && right > 0.8 * wide.thickness as f64);
        assert!(line.junctions.iter().all(|j| j.w == 450_000));
        // Still closed: it ends where it starts, on the seam.
        assert_eq!(line.junctions[0].p, mm(0.0, 0.0));
        assert_eq!(line.junctions[line.junctions.len() - 1].p, mm(0.0, 0.0));
    }

    #[test]
    fn supported_area_grows_the_layer_below() {
        let lower = [rect_region(0.0, 0.0, 10.0, 10.0)];
        let fuzzy = [
            everywhere(config(FuzzyType::External)),
            everywhere(FuzzyConfig {
                thickness: scaled(0.5),
                ..config(FuzzyType::Hole)
            }),
            // Not fuzzy: its thickness does not count.
            everywhere(FuzzyConfig {
                thickness: scaled(5.0),
                ..config(FuzzyType::None)
            }),
        ];
        let width = scaled(0.45);
        let area = supported_area(&fuzzy, 3, Some(&lower), width)
            .unwrap()
            .unwrap();
        assert_eq!(area.len(), 1);
        // Grown by the width and the largest fuzzy thickness: 0.95 mm.
        let side = 10.0 + 2.0 * 0.95;
        assert!(
            (area[0].area_mm2() - side * side).abs() < 1e-3,
            "{}",
            area[0].area_mm2()
        );

        // No layer below, nothing fuzzy, or the first layer: no area.
        assert!(supported_area(&fuzzy, 3, None, width).unwrap().is_none());
        assert!(
            supported_area(&fuzzy[2..], 3, Some(&lower), width)
                .unwrap()
                .is_none()
        );
        assert!(
            supported_area(&fuzzy, 0, Some(&lower), width)
                .unwrap()
                .is_none()
        );
        assert!(
            supported_area(&[], 3, Some(&lower), width)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn skip_overhangs_is_wanted_by_a_fuzzy_region_that_asks() {
        let asking = |kind| {
            everywhere(FuzzyConfig {
                skip_overhangs: true,
                ..config(kind)
            })
        };
        assert!(!skip_overhangs_wanted(&[]));
        assert!(!skip_overhangs_wanted(&[everywhere(config(
            FuzzyType::All
        ))]));
        assert!(skip_overhangs_wanted(&[
            everywhere(config(FuzzyType::All)),
            asking(FuzzyType::Hole)
        ]));
        assert!(!skip_overhangs_wanted(&[
            asking(FuzzyType::None),
            asking(FuzzyType::Disabled)
        ]));
    }
}
