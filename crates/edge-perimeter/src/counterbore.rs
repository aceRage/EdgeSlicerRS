//! A hole that narrows on the way up (a counterbore printed wide end
//! down): the first layer of the narrow part has the wide hole under it,
//! and its walls round the narrow hole would be laid in the air
//! (`counterbore_hole_bridging`).
//!
//! A port of `PerimeterGenerator::process_no_bridge`
//! (PerimeterGenerator.cpp), which the reference runs on a layer's
//! regions before it walls them, classic walls or Arachne. What of a
//! region has nothing under it and can be spanned is taken out of what
//! gets walls and handed to the fill as it is, to be laid as a bridge:
//!
//! - `partiallybridge`: only the part lines can span from one side of
//!   the wide hole to the other, which leaves a slot as wide as the
//!   narrow hole open right across the wide one. The rest of what has
//!   nothing under it is not printed on this layer at all; the layer
//!   above closes the slot down to the narrow hole, its lines resting on
//!   the two bridges.
//! - `sacrificiallayer`: all of it, the narrow hole covered over as
//!   well, to be drilled out afterwards.
//!
//! The walls of the layer then go round the wide hole, and the bridge
//! reaches a wall's width onto what is under them.

use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, Polygon, scaled};

use crate::{Error, Params, SAFETY_OFFSET, bridge_detector::BridgeDetector};

/// How far, in mm, a bridge reaches onto what holds it up at the most
/// (libslic3r.h `BRIDGE_INFILL_MARGIN`).
const BRIDGE_INFILL_MARGIN: f64 = 1.0;
/// A hole of the region within this of an area with nothing under it is
/// on that area's edge.
const TOUCHING: Coord = 10_000;
/// The reference's `SCALED_EPSILON`: lengths below it count as nothing.
const EPSILON: Coord = 100;

/// Which way a hole that narrows is bridged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CounterboreMode {
    /// `partiallybridge`.
    Partial,
    /// `sacrificiallayer`.
    Sacrificial,
}

/// What `counterbore_hole_bridging` asks, and the settings read with it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Counterbore {
    pub mode: CounterboreMode,
    /// The direction every bridge is to be laid in, in radians, where
    /// the profile fixes one (`bridge_angle` above zero without
    /// `relative_bridge_angle`): what can be spanned is then judged
    /// that way round alone.
    pub fixed_angle: Option<f64>,
    /// What is added to the direction found for a bridge, in radians
    /// (`bridge_angle` with `relative_bridge_angle`).
    pub added_angle: f64,
    /// Parts of the layer below narrower than this, in mm, are taken not
    /// to have been printed, and hold nothing up: a third of the
    /// nozzle's bore with classic walls, Arachne's least feature size
    /// with those (PerimeterGenerator.cpp `printable_slices`).
    pub unprintable: f64,
}

/// A layer's regions divided by [`split_counterbores`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Counterbores {
    /// What gets walls.
    pub walled: Vec<ExPolygon>,
    /// What is laid as a bridge and gets none. It lies outside `walled`,
    /// and with a sacrificial layer reaches over the hole, outside the
    /// regions altogether.
    pub bridged: Vec<ExPolygon>,
}

fn grown(regions: &[ExPolygon], by: Coord) -> Result<Vec<ExPolygon>, Error> {
    Ok(edge_clip::offset(regions, by, Join::MITER)?)
}

/// `diff_ex` with `ApplySafetyOffset::Yes`: what is cut away is grown by
/// a hundredth of a micron first, so that edges meant to coincide do
/// not leave slivers.
fn cut(subject: &[ExPolygon], clip: &[ExPolygon]) -> Result<Vec<ExPolygon>, Error> {
    Ok(edge_clip::difference(
        subject,
        &grown(clip, SAFETY_OFFSET)?,
    )?)
}

/// A hole's ring as the solid piece it leaves out.
fn plug(hole: &Polygon) -> ExPolygon {
    let mut contour = hole.clone();
    if !contour.is_ccw() {
        contour.points.reverse();
    }
    ExPolygon {
        contour,
        holes: Vec::new(),
    }
}

/// A piece's outline as the hole it makes in another.
fn as_hole(contour: &Polygon) -> Polygon {
    let mut hole = contour.clone();
    if hole.is_ccw() {
        hole.points.reverse();
    }
    hole
}

/// Whether a counter-clockwise ring turns left at every corner, a
/// corner counting as turning right only where it lies more than
/// [`EPSILON`] inside the line through its neighbours.
///
/// The reference counts any right turn at all
/// (`contour_simplified[0].concave_points().size() == 0`). Its outline
/// is simplified first, and simplifying keeps the outline's first point
/// whatever it is: where that point is in the middle of a straight side
/// it is left a nanometre off the side by rounding, to one side or the
/// other, and a hexagonal or square hole is then covered or not by
/// which. Not copied.
fn convex(ring: &Polygon) -> bool {
    let points = &ring.points;
    let count = points.len();
    (0..count).all(|at| {
        let (a, b, c) = (
            points[(at + count - 1) % count],
            points[at],
            points[(at + 1) % count],
        );
        // Twice the area of the corner's triangle: over the length of
        // its far side, how far the corner stands off that side.
        let cross = (i128::from(b.x - a.x) * i128::from(c.y - b.y)
            - i128::from(b.y - a.y) * i128::from(c.x - b.x)) as f64;
        let across = ((c.x - a.x) as f64).hypot((c.y - a.y) as f64);
        cross >= -(EPSILON as f64) * across
    })
}

/// Divides one filament's regions on a layer into what is walled and
/// what is bridged without walls, where a hole in them narrows from the
/// layer below. `lower` is the whole of the layer below, every filament
/// together. Regions with nothing of the kind come back as they are and
/// in their order.
pub fn split_counterbores(
    regions: &[ExPolygon],
    lower: &[ExPolygon],
    params: &Params,
    bore: &Counterbore,
) -> Result<Counterbores, Error> {
    let join = Join::MITER;
    let spacing = params.inner.scaled_spacing();
    let ext_width = params.outer.scaled_width();
    let margin = scaled(BRIDGE_INFILL_MARGIN);
    // How far a bridge reaches onto what holds it up.
    let anchor = margin.min(spacing).min(ext_width);
    let mut split = Counterbores {
        walled: regions.to_vec(),
        bridged: Vec::new(),
    };
    if lower.is_empty() || regions.is_empty() {
        return Ok(split);
    }
    // "less what the lower layer could not print"
    let unprintable = scaled(bore.unprintable / 2.0);
    let lower = if unprintable > 0 {
        edge_clip::opening(lower, unprintable, join)?
    } else {
        lower.to_vec()
    };
    let surfaces = &mut split.walled;
    let mut at = 0;
    while at < surfaces.len() {
        let surface = surfaces[at].clone();
        let mut last = vec![surface.clone()];
        let unsupported = cut(&last, &lower)?;
        // "remove small overhangs"
        let mut filtered = if unsupported.is_empty() {
            Vec::new()
        } else {
            edge_clip::opening(&unsupported, spacing, join)?
        };
        // Only where a hole of the region is on the edge of what has
        // nothing under it: a hole that narrows. The reference asks no
        // more than that the region has a hole somewhere
        // (PerimeterGenerator.cpp, "keep out if cannot be bridged or no
        // holes to bridge": `!surface->expolygon.holes.empty()`), so
        // there a deck across two posts loses the walls of its first
        // layer, and whatever of that layer cannot be spanned, when a
        // hole is drilled anywhere else in it. Not copied.
        if !filtered.is_empty() {
            let holes: Vec<ExPolygon> = surface.holes.iter().map(plug).collect();
            let beside = grown(&holes, TOUCHING)?;
            let mut kept = Vec::with_capacity(filtered.len());
            for island in filtered {
                if !beside.is_empty()
                    && !edge_clip::intersection(std::slice::from_ref(&island), &beside)?.is_empty()
                {
                    kept.push(island);
                }
            }
            filtered = kept;
        }
        if !filtered.is_empty() {
            // "extract only the useful part of the lower layer. The
            // safety offset is really needed here."
            let support = cut(&last, &unsupported)?;
            // "only consider the part that can be bridged (really, by
            // the bridge algorithm)": a detector for each island, with
            // lines a quarter of a wall apart.
            let mut bridgeable: Vec<ExPolygon> = Vec::new();
            for island in &filtered {
                let detector = BridgeDetector::new(island, &support, spacing / 4)?;
                if let Some(angle) = detector.detect_angle(bore.fixed_angle)? {
                    bridgeable.extend(detector.coverage(angle)?);
                }
            }
            if bridgeable.is_empty() {
                filtered.clear();
            } else if bore.mode == CounterboreMode::Sacrificial {
                // "shrink it to survive the strict bridge-candidate
                // filter"
                let shrunk = grown(&filtered, -spacing)?;
                let reach = grown(&bridgeable, margin + spacing / 2)?;
                let allowed =
                    edge_clip::union_ex(&last.iter().chain(&reach).cloned().collect::<Vec<_>>())?;
                // "we bridge everything, even the not-bridgeable bits":
                // every island whose outline is convex, that has some
                // bridge in it, and that is not an overhang at the
                // region's edge.
                let mut kept = Vec::new();
                for island in shrunk {
                    let outline = ExPolygon {
                        contour: island.contour.clone(),
                        holes: Vec::new(),
                    };
                    let own = std::slice::from_ref(&outline);
                    let simplified = edge_clip::simplify(own, spacing)?;
                    let bigger = grown(own, margin)?;
                    let ([simple], [bigger]) = (simplified.as_slice(), bigger.as_slice()) else {
                        continue;
                    };
                    if !simple.holes.is_empty() || !convex(&simple.contour) {
                        continue;
                    }
                    if edge_clip::intersection(&bridgeable, std::slice::from_ref(&island))?
                        .is_empty()
                    {
                        continue;
                    }
                    let bigger = ExPolygon {
                        contour: bigger.contour.clone(),
                        holes: island.holes.clone(),
                    };
                    if !cut(&[bigger], &allowed)?.is_empty() {
                        continue;
                    }
                    kept.push(island);
                }
                // "restore it back to its original size and add anchor",
                // "clamp to the original surface, to avoid creating new
                // unsupported areas"
                filtered = edge_clip::intersection(&last, &grown(&kept, spacing + anchor)?)?;
                for cover in &mut filtered {
                    // "Remove holes that need sacrificial fill, but keep
                    // holes whose wall is already supported by the lower
                    // layer."
                    let wall = ext_width / 2;
                    let mut hole = 0;
                    while hole < cover.holes.len() {
                        let inside = [plug(&cover.holes[hole])];
                        let around = edge_clip::intersection(
                            &cut(&grown(&inside, wall)?, &inside)?,
                            &grown(std::slice::from_ref(cover), SAFETY_OFFSET)?,
                        )?;
                        if !around.is_empty()
                            && edge_clip::intersection(&around, &grown(&lower, SAFETY_OFFSET)?)?
                                .is_empty()
                        {
                            cover.holes.remove(hole);
                        } else {
                            hole += 1;
                        }
                    }
                    // "detect inside volume": a piece of the layer that
                    // stood in a hole now covered is left its place, less
                    // the margin the cover rests on it by.
                    let mut other = 0;
                    while other < surfaces.len() {
                        if other == at
                            || edge_clip::intersection(
                                std::slice::from_ref(cover),
                                std::slice::from_ref(&surfaces[other]),
                            )?
                            .is_empty()
                        {
                            other += 1;
                            continue;
                        }
                        let left = edge_clip::offset2(
                            std::slice::from_ref(&surfaces[other]),
                            -margin - spacing,
                            spacing,
                            join,
                        )?;
                        cover
                            .holes
                            .extend(left.iter().map(|piece| as_hole(&piece.contour)));
                        if let [one] = left.as_slice() {
                            surfaces[other] = one.clone();
                            other += 1;
                        } else {
                            surfaces.extend(left);
                            surfaces.remove(other);
                            if other < at {
                                at -= 1;
                            }
                        }
                    }
                }
            } else {
                // "Partial counterbore bridging is mask-based. Preserve
                // the supported remainder and use simplified
                // BridgeDetector coverage to derive the bridgeable
                // counterbore span. The span is grown from supported
                // material, shrunk back, stripped from the remaining
                // normal surface, and expanded back."
                let remaining = cut(&last, &filtered)?;
                let mut span = edge_clip::opening(
                    &edge_clip::simplify(&bridgeable, spacing)?,
                    ext_width,
                    join,
                )?;
                // "Get rid of coarseness of the resulted bridgeable area
                // by using the original supported area as reference."
                span.extend(grown(&remaining, spacing)?);
                span = grown(&edge_clip::union_ex(&span)?, -spacing)?;
                span = cut(&span, &remaining)?;
                // "filter noise from the diff_ex"
                span = edge_clip::opening(&span, ext_width / 2, join)?;
                span = grown(&span, spacing)?;
                // "Keep the bridge mask from intruding deeper into the
                // supported anchor region than the explicit anchor
                // overlap."
                span = edge_clip::difference(&span, &grown(&remaining, -anchor)?)?;
                let anchors = edge_clip::intersection(&remaining, &grown(&filtered, anchor)?)?;
                span.extend(anchors);
                // "remove anchor area from hole-side walls, it must
                // remain unbridgeable"
                filtered = edge_clip::opening(&edge_clip::union_ex(&span)?, anchor, join)?;
                // "update 'last' only if we have a valid bridgeable
                // area, otherwise we will lose the original unsupported
                // area"
                if !filtered.is_empty() {
                    last = remaining;
                }
            }
            if !filtered.is_empty() {
                // "add this directly to the infill list. this will avoid
                // to throw wrong offsets into a good polygons"
                last = cut(&last, &filtered)?;
                split.bridged.append(&mut filtered);
                // "remove "thin air" polygons": what is left with
                // nothing of the supported part in it is filled too.
                let mut kept = Vec::with_capacity(last.len());
                for piece in last {
                    if edge_clip::intersection(&support, std::slice::from_ref(&piece))?.is_empty() {
                        split.bridged.push(piece);
                    } else {
                        kept.push(piece);
                    }
                }
                last = kept;
            }
        }
        // The first piece keeps the region's place; the rest go to the
        // end, and are looked at in their turn.
        let mut pieces = last.into_iter();
        match pieces.next() {
            Some(first) => {
                surfaces[at] = first;
                surfaces.extend(pieces);
                at += 1;
            }
            None => {
                surfaces.remove(at);
            }
        }
    }
    Ok(split)
}

#[cfg(test)]
mod tests {
    use edge_geom::Point2;

    use super::*;
    use crate::WallSettings;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn solid(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: rect(x0, y0, x1, y1),
            holes: Vec::new(),
        }
    }

    /// A slab 30 mm square about the origin with these holes in it.
    fn slab(holes: &[Polygon]) -> ExPolygon {
        ExPolygon {
            contour: rect(-15.0, -15.0, 15.0, 15.0),
            holes: holes
                .iter()
                .map(|hole| {
                    let mut hole = hole.clone();
                    hole.points.reverse();
                    hole
                })
                .collect(),
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    fn overlap(a: &[ExPolygon], b: &[ExPolygon]) -> f64 {
        area(&edge_clip::intersection(a, b).unwrap())
    }

    fn params() -> Params {
        WallSettings {
            arachne: None,
            ..WallSettings::default()
        }
        .params(0.2, false)
        .unwrap()
    }

    fn bore(mode: CounterboreMode) -> Counterbore {
        Counterbore {
            mode,
            fixed_angle: None,
            added_angle: 0.0,
            unprintable: 0.4 / 3.0,
        }
    }

    fn split(layer: &[ExPolygon], lower: &[ExPolygon], mode: CounterboreMode) -> Counterbores {
        split_counterbores(layer, lower, &params(), &bore(mode)).unwrap()
    }

    /// A hole 4 mm square over one 12 mm by 8 mm: the wide hole is
    /// longer in x, so that lines along y span it best.
    fn narrowing() -> (Vec<ExPolygon>, Vec<ExPolygon>) {
        (
            vec![slab(&[rect(-2.0, -2.0, 2.0, 2.0)])],
            vec![slab(&[rect(-6.0, -4.0, 6.0, 4.0)])],
        )
    }

    #[test]
    fn a_hole_straight_through_is_left_alone() {
        let layer = vec![slab(&[rect(-2.0, -2.0, 2.0, 2.0)])];
        for mode in [CounterboreMode::Partial, CounterboreMode::Sacrificial] {
            let made = split(&layer, &layer, mode);
            assert_eq!(made.walled, layer);
            assert!(made.bridged.is_empty());
        }
        // Nor is anything done on the bed, or to nothing.
        let made = split(&layer, &[], CounterboreMode::Partial);
        assert_eq!(made.walled, layer);
        assert!(
            split(&[], &layer, CounterboreMode::Partial)
                .walled
                .is_empty()
        );
    }

    #[test]
    fn partly_bridged_leaves_a_slot_as_wide_as_the_narrow_hole() {
        let (layer, lower) = narrowing();
        let made = split(&layer, &lower, CounterboreMode::Partial);
        let p = params();
        let anchor = edge_geom::unscaled(p.inner.scaled_spacing().min(p.outer.scaled_width()));
        // Two bridges, one each side of the narrow hole in x.
        assert_eq!(made.bridged.len(), 2, "{:?}", made.bridged);
        let left = solid(-6.0 - anchor, -4.0 - anchor, -2.0, 4.0 + anchor);
        let right = solid(2.0, -4.0 - anchor, 6.0 + anchor, 4.0 + anchor);
        for side in [&left, &right] {
            let covered = overlap(&made.bridged, std::slice::from_ref(side));
            // All of that side but a strip of the detector's along the hole.
            let whole = side.area_mm2();
            assert!(
                covered > whole - 0.5 * (8.0 + 2.0 * anchor) && covered < whole + 1e-6,
                "{covered} of {whole}"
            );
        }
        // The slot stays open from one end of the wide hole to the
        // other: nothing is bridged over the narrow hole, and next to
        // nothing above and below it.
        let slot = solid(-1.5, -4.0, 1.5, 4.0);
        assert!(overlap(&made.bridged, &[slot]) < 1e-6);
        // A bridge rests on the layer below by no more than the anchor.
        let resting = overlap(&made.bridged, &lower);
        let most = 2.0 * anchor * (4.0 + 8.0 + 4.0 + 4.0 * anchor);
        assert!(resting > 0.5 * most && resting < most + 0.01, "{resting}");
        // What is walled has nothing of the wide hole's roof in it, and
        // nothing of the bridges.
        assert!(area(&edge_clip::difference(&made.walled, &lower).unwrap()) < 0.01);
        assert!(overlap(&made.walled, &made.bridged) < 1e-3);
        // It is the slab less the wide hole, bitten into by the anchors.
        let supported = area(&lower);
        let walled = area(&made.walled);
        assert!(
            walled < supported && walled > supported - most - 0.01,
            "{walled}"
        );
    }

    #[test]
    fn a_sacrificial_layer_covers_the_narrow_hole_as_well() {
        let (layer, lower) = narrowing();
        let made = split(&layer, &lower, CounterboreMode::Sacrificial);
        let p = params();
        let anchor = edge_geom::unscaled(p.inner.scaled_spacing().min(p.outer.scaled_width()));
        assert_eq!(made.bridged.len(), 1, "{:?}", made.bridged);
        let cover = &made.bridged[0];
        assert!(cover.holes.is_empty());
        // The wide hole and the anchor round it, hole and all.
        let whole = (12.0 + 2.0 * anchor) * (8.0 + 2.0 * anchor);
        assert!(
            (cover.area_mm2() - whole).abs() < 0.01,
            "{}",
            cover.area_mm2()
        );
        assert!(overlap(&made.bridged, &[solid(-2.0, -2.0, 2.0, 2.0)]) > 16.0 - 1e-6);
        // The walls go round the cover.
        assert_eq!(made.walled.len(), 1);
        assert!((area(&made.walled) - (900.0 - whole)).abs() < 0.01);
        assert_eq!(made.walled[0].holes.len(), 1);
    }

    #[test]
    fn a_wide_hole_that_is_not_convex_gets_no_sacrificial_layer() {
        // An L: 12 by 8 with a corner 6 by 4 left solid.
        let wide = Polygon {
            points: [
                (-6.0, -4.0),
                (6.0, -4.0),
                (6.0, 0.0),
                (0.0, 0.0),
                (0.0, 4.0),
                (-6.0, 4.0),
            ]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
        };
        let layer = vec![slab(&[rect(-4.0, -3.0, -2.0, -1.0)])];
        let lower = vec![slab(&[wide])];
        let made = split(&layer, &lower, CounterboreMode::Sacrificial);
        assert_eq!(made.walled, layer);
        assert!(made.bridged.is_empty());
        // Partly bridged does not ask for that.
        let made = split(&layer, &lower, CounterboreMode::Partial);
        assert!(!made.bridged.is_empty());
    }

    #[test]
    fn a_cover_is_left_open_over_what_stands_in_the_wide_hole() {
        // A post 3 mm square stands in the wide hole, up to the layer
        // below; the narrow hole is beside it.
        let layer = vec![slab(&[rect(2.0, -1.0, 4.0, 1.0)])];
        let mut lower = vec![slab(&[rect(-6.0, -4.0, 6.0, 4.0)])];
        lower.push(solid(-4.5, -1.5, -1.5, 1.5));
        let made = split(&layer, &lower, CounterboreMode::Sacrificial);
        assert_eq!(made.bridged.len(), 1, "{:?}", made.bridged);
        // The narrow hole is covered; the middle of the post is not,
        // only its rim, which the cover rests on.
        assert_eq!(made.bridged[0].holes.len(), 1);
        assert!(overlap(&made.bridged, &[solid(2.0, -1.0, 4.0, 1.0)]) > 4.0 - 1e-6);
        assert!(overlap(&made.bridged, &[solid(-3.5, -0.5, -2.5, 0.5)]) < 1e-6);
        let on_post = overlap(&made.bridged, &[solid(-4.5, -1.5, -1.5, 1.5)]);
        assert!(on_post > 1.0 && on_post < 9.0 - 1.0, "{on_post}");
        // What is over the middle of the post is walled as it was.
        assert!(overlap(&made.walled, &[solid(-3.5, -0.5, -2.5, 0.5)]) > 1.0 - 1e-6);
    }

    #[test]
    fn an_overhang_with_no_hole_on_its_edge_is_not_a_counterbore() {
        // A deck across two posts, with a hole down into one of the
        // posts: the span has no hole on its edge, and keeps its walls.
        let mut hole = rect(-13.0, -1.0, -11.0, 1.0);
        hole.points.reverse();
        let deck = vec![ExPolygon {
            contour: rect(-15.0, -5.0, 15.0, 5.0),
            holes: vec![hole],
        }];
        let posts = vec![solid(-15.0, -5.0, -9.0, 5.0), solid(9.0, -5.0, 15.0, 5.0)];
        for mode in [CounterboreMode::Partial, CounterboreMode::Sacrificial] {
            let made = split(&deck, &posts, mode);
            assert_eq!(made.walled, deck, "{mode:?}");
            assert!(made.bridged.is_empty());
        }
        // With the hole in the span it is one: the span either side of
        // the hole is bridged without walls.
        let mut hole = rect(-1.0, -1.0, 1.0, 1.0);
        hole.points.reverse();
        let deck = vec![ExPolygon {
            contour: rect(-15.0, -5.0, 15.0, 5.0),
            holes: vec![hole],
        }];
        let made = split(&deck, &posts, CounterboreMode::Partial);
        assert_eq!(made.bridged.len(), 2, "{:?}", made.bridged);
    }

    #[test]
    fn one_direction_for_every_bridge_turns_the_slot() {
        let (layer, lower) = narrowing();
        // Along x, the long way: the slot then runs along x.
        let along_x = Counterbore {
            fixed_angle: Some(0.0001),
            ..bore(CounterboreMode::Partial)
        };
        let made = split_counterbores(&layer, &lower, &params(), &along_x).unwrap();
        assert_eq!(made.bridged.len(), 2, "{:?}", made.bridged);
        assert!(overlap(&made.bridged, &[solid(-6.0, -1.5, 6.0, 1.5)]) < 1e-6);
        assert!(overlap(&made.bridged, &[solid(-6.0, 2.6, 6.0, 4.0)]) > 12.0 * 1.4 - 0.5);
    }

    #[test]
    fn a_corner_a_nanometre_out_of_line_is_no_dent() {
        // A hexagon whose first point is the middle of a side, one unit
        // inside it.
        let corner = |turn: f64| {
            let angle = turn * std::f64::consts::FRAC_PI_3;
            Point2::from_mm(5.0 * angle.cos(), 5.0 * angle.sin())
        };
        let (a, b) = (corner(0.0), corner(1.0));
        let mut points = vec![Point2 {
            x: (a.x + b.x) / 2 - 1,
            y: (a.y + b.y) / 2 - 1,
        }];
        points.extend((1..=6).map(|turn| corner(f64::from(turn))));
        assert!(convex(&Polygon { points }));
        // A real dent is one.
        let mut dented = rect(0.0, 0.0, 10.0, 10.0);
        dented.points.insert(1, Point2::from_mm(5.0, 0.01));
        assert!(!convex(&dented));
        assert!(convex(&rect(0.0, 0.0, 10.0, 10.0)));
    }
}
