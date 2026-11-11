//! Bridges over infill as the reference's options shape them:
//! `dont_filter_internal_bridges`, which says how small a patch of solid
//! fill over sparse fill is still worth bridging
//! (PrintObject.cpp `PrintObject::bridge_over_infill`).

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon};

use crate::surfaces::{Beneath, Surface, SurfaceKind, bridge_over_infill};

/// How small bridges over infill are sorted out
/// (`dont_filter_internal_bridges`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InternalBridgeFilter {
    /// `disabled`, the default: small ones are left as solid fill laid
    /// straight on the sparse fill.
    Filter,
    /// `limited`: solid fill is taken to hold up what is within two line
    /// spacings of it where the default takes four, and sparse fill
    /// counts from one spacing inside its edge where the default counts
    /// from three.
    Limited,
    /// `nofilter`: as `limited`, and every patch found is bridged however
    /// small, with four spacings round it.
    NoFilter,
}

/// What picking out the bridges over infill needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverInfill {
    /// The distance between solid lines.
    pub spacing: Coord,
    pub filter: InternalBridgeFilter,
    /// The distance between the round threads a thick bridge is laid in
    /// (`bridging_flow(frSolidInfill, true)`), which the reference
    /// measures a bridge's surroundings by.
    pub thread_spacing: Coord,
    /// How many layers down sparse fill is looked for: those a thread of
    /// a thick bridge would sink into, and never fewer than one.
    pub sparse_depth: usize,
    /// How many layers down a bridge already made keeps another from
    /// being laid over it.
    pub bridged_depth: usize,
}

/// Edges meant to coincide are made to (the reference's `SCALED_EPSILON`).
const SNAP: Coord = 100;

/// [`bridge_over_infill`] for every filament (`surfaces[f][i]` is
/// filament `f`'s layer `i`), sorting small bridges out as `over.filter`
/// says. With the default filter this is that function and no more.
///
/// With either other filter the reference's second sorting-out is done
/// as well, which the default has no need of here: a patch is bridged
/// only where, grown by a thread's spacing, it reaches sparse fill three
/// spacings inside that fill's own edge (`bridge_components` against
/// `internal_unsupported_area`), and not over a bridge made on a layer a
/// thread's height or less beneath it. Layers are therefore taken in
/// order, each knowing what the ones under it came to.
///
/// Different from the reference: there a patch kept is then run out, the
/// way its lines will go, to the sparse lines that carry it, which is
/// not done here for any bridge over infill, so a small patch comes out
/// smaller here than there, and the bridge it rules out on the layer
/// above is sometimes laid here. With limited filtering a bridge stays
/// within the solid fill it was found in, as it does here with the
/// default. And two patches the reference would let overlap are kept
/// apart, the first to come keeping what both would cover.
pub fn bridge_over_infill_by(
    surfaces: &[Vec<Vec<Surface>>],
    beneath: &[Beneath],
    over: &OverInfill,
) -> Result<Vec<Vec<Vec<Surface>>>, ClipError> {
    if over.filter == InternalBridgeFilter::Filter {
        return surfaces
            .iter()
            .map(|own| bridge_over_infill(own, beneath, over.spacing))
            .collect();
    }
    for own in surfaces {
        assert_eq!(own.len(), beneath.len(), "one layer beneath per layer");
    }
    let join = Join::MITER;
    let (spacing, thread) = (over.spacing, over.thread_spacing);
    let area_of = |regions: &[ExPolygon]| regions.iter().map(ExPolygon::area_mm2).sum::<f64>();
    let unit = edge_geom::unscaled(spacing);
    let mut out: Vec<Vec<Vec<Surface>>> = surfaces
        .iter()
        .map(|own| Vec::with_capacity(own.len()))
        .collect();
    // The bridges made on each layer, over every filament.
    let mut made: Vec<Vec<ExPolygon>> = Vec::with_capacity(beneath.len());
    for layer in 0..beneath.len() {
        let mut bridges: Vec<ExPolygon> = Vec::new();
        let unsupported = if layer == 0
            || !surfaces.iter().any(|own| {
                own[layer]
                    .iter()
                    .any(|s| s.kind == SurfaceKind::InternalSolid)
            }) {
            Vec::new()
        } else {
            // As with the default, with one spacing where that has
            // three (`expansion_multiplier`).
            let below = &beneath[layer - 1];
            let footing = edge_clip::offset(
                &edge_clip::offset(&below.solid, -spacing, join)?,
                2 * spacing,
                join,
            )?;
            let open = edge_clip::offset(
                &edge_clip::closing(&below.infill, SNAP, join)?,
                -spacing,
                join,
            )?;
            edge_clip::difference(&open, &footing)?
        };
        if unsupported.is_empty() {
            for (own, changed) in surfaces.iter().zip(&mut out) {
                changed.push(own[layer].clone());
            }
            made.push(bridges);
            continue;
        }
        // Where a thick bridge's threads have sparse fill to sink into
        // (`gather_areas_w_depth`): sparse on the layers a thread is deep,
        // none of them anything else, and not bridged already on a layer
        // within a thread's height.
        let (mut sparse, mut dense) = (Vec::new(), Vec::new());
        for under in beneath[layer.saturating_sub(over.sparse_depth)..layer].iter() {
            sparse.extend(edge_clip::difference(&under.infill, &under.solid)?);
            dense.extend(under.solid.iter().cloned());
        }
        let mut deep = edge_clip::difference(
            &edge_clip::closing(&sparse, SNAP, join)?,
            &edge_clip::closing(&dense, SNAP, join)?,
        )?;
        for bridged in &made[layer.saturating_sub(over.bridged_depth)..layer] {
            if !bridged.is_empty() {
                deep = edge_clip::difference(&deep, bridged)?;
            }
        }
        let deep = edge_clip::offset(&deep, (1.5 * thread as f64) as Coord, join)?;
        let inner = edge_clip::offset(&deep, -((4.5 * thread as f64) as Coord), join)?;
        // The pieces of a patch worth bridging: those that, a thread's
        // spacing larger and kept to where there is sparse fill to
        // bridge, reach well inside that fill.
        let carried = |patch: &[ExPolygon]| -> Result<Vec<ExPolygon>, ClipError> {
            if patch.is_empty() || inner.is_empty() {
                return Ok(Vec::new());
            }
            let mut kept = Vec::new();
            for piece in edge_clip::intersection(&edge_clip::offset(patch, thread, join)?, &deep)? {
                let own = std::slice::from_ref(&piece);
                if !edge_clip::intersection(own, &inner)?.is_empty() {
                    kept.push(piece);
                }
            }
            Ok(kept)
        };
        for (own, changed) in surfaces.iter().zip(&mut out) {
            let own = &own[layer];
            let mut new = Vec::with_capacity(own.len());
            if over.filter == InternalBridgeFilter::Limited {
                for surface in own {
                    if surface.kind != SurfaceKind::InternalSolid {
                        new.push(surface.clone());
                        continue;
                    }
                    // The default's own rule for what is worth bridging.
                    let whole = std::slice::from_ref(&surface.area);
                    let over = edge_clip::intersection(whole, &unsupported)?;
                    let partly = area_of(&over) < surface.area.area_mm2() - 1e-4;
                    if over.is_empty() || (partly && area_of(&over) <= 9.0 * unit * unit) {
                        new.push(surface.clone());
                        continue;
                    }
                    let mut bridged = edge_clip::intersection(
                        whole,
                        &edge_clip::offset(&over, 4 * spacing, join)?,
                    )?;
                    let beside =
                        edge_clip::difference(whole, &edge_clip::offset(&bridged, spacing, join)?)?;
                    bridged.extend(beside.into_iter().filter(|scrap| {
                        let area = scrap.area_mm2();
                        area > unit * unit && area < unit * 12.0
                    }));
                    let bridged =
                        edge_clip::intersection(&edge_clip::closing(&bridged, SNAP, join)?, whole)?;
                    // What of it has sparse fill to bridge is kept, and
                    // the rest left as solid fill.
                    let kept = edge_clip::intersection(&bridged, &carried(&bridged)?)?;
                    if kept.is_empty() {
                        new.push(surface.clone());
                        continue;
                    }
                    let rest = edge_clip::difference(whole, &kept)?;
                    bridges.extend(kept.iter().cloned());
                    new.extend(
                        kept.into_iter()
                            .map(|area| Surface::new(SurfaceKind::InternalBridge, area)),
                    );
                    new.extend(
                        rest.into_iter()
                            .map(|area| Surface::new(SurfaceKind::InternalSolid, area)),
                    );
                }
            } else {
                // Every patch, with four spacings round it wherever that
                // falls: on solid fill or sparse, the patch's own surface
                // or another.
                let fill: Vec<ExPolygon> = own
                    .iter()
                    .filter(|s| matches!(s.kind, SurfaceKind::InternalSolid | SurfaceKind::Sparse))
                    .map(|s| s.area.clone())
                    .collect();
                let mut taken: Vec<ExPolygon> = Vec::new();
                let mut patches: Vec<Vec<ExPolygon>> = Vec::with_capacity(own.len());
                for surface in own {
                    let mut patch = Vec::new();
                    if surface.kind == SurfaceKind::InternalSolid {
                        let whole = std::slice::from_ref(&surface.area);
                        let over = edge_clip::intersection(whole, &unsupported)?;
                        if !over.is_empty() {
                            // Without the scraps a thread and a half
                            // could not cover, and with the gaps a
                            // thread would not fit closed (the
                            // reference's last touches to a bridge).
                            let reach = carried(&edge_clip::offset(&over, 4 * spacing, join)?)?;
                            let tidy = edge_clip::closing(
                                &edge_clip::opening(&reach, (0.75 * thread as f64) as Coord, join)?,
                                thread,
                                join,
                            )?;
                            patch = edge_clip::intersection(
                                &edge_clip::intersection(&tidy, &deep)?,
                                &fill,
                            )?;
                            if !taken.is_empty() {
                                patch = edge_clip::difference(&patch, &taken)?;
                            }
                            taken.extend(patch.iter().cloned());
                        }
                    }
                    patches.push(patch);
                }
                for (surface, patch) in own.iter().zip(patches) {
                    let fills = matches!(
                        surface.kind,
                        SurfaceKind::InternalSolid | SurfaceKind::Sparse
                    );
                    if !fills || taken.is_empty() {
                        new.push(surface.clone());
                        continue;
                    }
                    new.extend(
                        patch
                            .into_iter()
                            .map(|area| Surface::new(SurfaceKind::InternalBridge, area)),
                    );
                    let whole = std::slice::from_ref(&surface.area);
                    new.extend(
                        edge_clip::difference(whole, &taken)?
                            .into_iter()
                            .map(|area| Surface::new(surface.kind, area)),
                    );
                }
                bridges.extend(taken);
            }
            changed.push(new);
        }
        made.push(bridges);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use edge_geom::{Point2, Polygon, scaled};

    use super::*;

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn over(filter: InternalBridgeFilter) -> OverInfill {
        OverInfill {
            spacing: scaled(0.4),
            filter,
            thread_spacing: scaled(0.4),
            sparse_depth: 1,
            bridged_depth: 1,
        }
    }

    fn area(layer: &[Surface], kind: SurfaceKind) -> f64 {
        layer
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.area.area_mm2())
            .sum()
    }

    /// A 30 mm square of sparse fill, and over it the same square with a
    /// strip of solid fill `width` wide from `from` in from its left
    /// edge, the rest sparse. Returns the upper layer as each filter
    /// leaves it.
    fn strip(from: f64, width: f64, filter: InternalBridgeFilter) -> Vec<Surface> {
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let solid = block(from, 0.0, from + width, 30.0);
        let mut upper = vec![Surface::new(SurfaceKind::InternalSolid, solid.clone())];
        upper.extend(
            edge_clip::difference(std::slice::from_ref(&whole), std::slice::from_ref(&solid))
                .unwrap()
                .into_iter()
                .map(|area| Surface::new(SurfaceKind::Sparse, area)),
        );
        let lower = vec![Surface::new(SurfaceKind::Sparse, whole.clone())];
        let beneath = [
            Beneath::gather([(lower.as_slice(), std::slice::from_ref(&whole))]),
            Beneath::default(),
        ];
        bridge_over_infill_by(&[vec![lower, upper]], &beneath, &over(filter))
            .unwrap()
            .remove(0)
            .remove(1)
    }

    use InternalBridgeFilter::{Filter, Limited, NoFilter};
    use SurfaceKind::{InternalBridge, InternalSolid, Sparse};

    #[test]
    fn the_default_is_the_filter_as_it_was() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole.clone())];
        let stack = vec![of(Sparse), of(InternalSolid)];
        let beneath = [
            Beneath::gather([(stack[0].as_slice(), std::slice::from_ref(&whole))]),
            Beneath::default(),
        ];
        let found =
            bridge_over_infill_by(std::slice::from_ref(&stack), &beneath, &over(Filter)).unwrap();
        assert_eq!(
            found[0],
            bridge_over_infill(&stack, &beneath, scaled(0.4)).unwrap()
        );
        // A whole layer of solid over sparse fill is a bridge whatever
        // the filter.
        for filter in [Limited, NoFilter] {
            let found =
                bridge_over_infill_by(std::slice::from_ref(&stack), &beneath, &over(filter))
                    .unwrap();
            assert!((area(&found[0][1], InternalBridge) - 400.0).abs() < 1e-3);
            assert_eq!(area(&found[0][1], InternalSolid), 0.0);
        }
    }

    #[test]
    fn a_strip_along_the_wall_is_bridged_only_with_less_filtering() {
        // Solid fill from 0.6 to 2.2 mm in from the wall: the default
        // counts sparse fill from 1.2 mm in and finds 1 mm of it under
        // the strip, 30 mm² of 48 and more than nine spacings square, so
        // the strip is bridged, and with less filtering as well.
        for filter in [Filter, Limited, NoFilter] {
            assert!(
                area(&strip(0.6, 1.6, filter), InternalBridge) > 40.0,
                "{filter:?}"
            );
        }
        // From 0.2 to 1.0 mm in, the default finds nothing under it.
        // Counting from 0.4 mm in there is 0.6 mm under it, 18 mm²: both
        // other filters bridge it.
        let narrow = |filter| strip(0.2, 0.8, filter);
        assert_eq!(area(&narrow(Filter), InternalBridge), 0.0);
        assert!((area(&narrow(Limited), InternalBridge) - 24.0).abs() < 1e-3);
        assert!((area(&narrow(Limited), InternalSolid)).abs() < 1e-3);
        // With no filter it takes four spacings round what it found and
        // a thread's spacing more, of the sparse fill beside it as well:
        // from the wall to 1.0 + 1.6 + 0.4 mm in.
        let all = narrow(NoFilter);
        assert!((area(&all, InternalBridge) - 30.0 * 3.0).abs() < 0.5);
        assert!((area(&all, Sparse) - 30.0 * 27.0).abs() < 0.5);
        assert_eq!(area(&all, InternalSolid), 0.0);
        // The layer is still covered once.
        let total: f64 = all.iter().map(|s| s.area.area_mm2()).sum();
        assert!((total - 900.0).abs() < 1e-3, "{total}");
    }

    #[test]
    fn a_sliver_is_bridged_only_with_no_filter() {
        // Solid fill from 0.2 to 0.44 mm in: 0.04 mm of it over sparse
        // fill counted from 0.4 mm, 1.2 mm² and under nine spacings
        // square. Limited filtering leaves it; no filtering bridges it.
        let sliver = |filter| area(&strip(0.2, 0.24, filter), InternalBridge);
        assert_eq!(sliver(Filter), 0.0);
        assert_eq!(sliver(Limited), 0.0);
        assert!(sliver(NoFilter) > 30.0, "{}", sliver(NoFilter));
    }

    #[test]
    fn nothing_is_bridged_where_the_sparse_fill_beneath_is_too_narrow() {
        // A strip of solid over a band of sparse fill 2 mm wide between
        // two solid areas: no thread could sink three spacings into it.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let band = block(14.0, 0.0, 16.0, 30.0);
        let lower: Vec<Surface> = std::iter::once(Surface::new(Sparse, band.clone()))
            .chain(
                edge_clip::difference(std::slice::from_ref(&whole), std::slice::from_ref(&band))
                    .unwrap()
                    .into_iter()
                    .map(|area| Surface::new(InternalSolid, area)),
            )
            .collect();
        let upper = vec![Surface::new(InternalSolid, whole.clone())];
        let beneath = [
            Beneath::gather([(lower.as_slice(), std::slice::from_ref(&whole))]),
            Beneath::default(),
        ];
        for filter in [Limited, NoFilter] {
            let found = bridge_over_infill_by(
                &[vec![lower.clone(), upper.clone()]],
                &beneath,
                &over(filter),
            )
            .unwrap();
            assert_eq!(area(&found[0][1], InternalBridge), 0.0, "{filter:?}");
            assert_eq!(found[0][1], upper);
        }
    }

    #[test]
    fn a_bridge_is_not_laid_over_one_just_beneath_it() {
        // Three layers: sparse, then a narrow strip of solid along the
        // wall on each of the next two, the upper one a little further
        // in. With no filter the first becomes a bridge with four
        // spacings round it; the second, which would lie over that
        // bridge, is left as it was.
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let layer = |from: f64, width: f64| -> Vec<Surface> {
            let solid = block(from, 0.0, from + width, 30.0);
            std::iter::once(Surface::new(InternalSolid, solid.clone()))
                .chain(
                    edge_clip::difference(
                        std::slice::from_ref(&whole),
                        std::slice::from_ref(&solid),
                    )
                    .unwrap()
                    .into_iter()
                    .map(|area| Surface::new(Sparse, area)),
                )
                .collect()
        };
        let stack = vec![
            vec![Surface::new(Sparse, whole.clone())],
            layer(0.2, 0.3),
            layer(0.3, 0.3),
        ];
        let beneath: Vec<Beneath> = stack
            .iter()
            .map(|surfaces| Beneath::gather([(surfaces.as_slice(), std::slice::from_ref(&whole))]))
            .collect();
        let found =
            bridge_over_infill_by(std::slice::from_ref(&stack), &beneath, &over(NoFilter)).unwrap();
        assert!(area(&found[0][1], InternalBridge) > 30.0);
        assert_eq!(area(&found[0][2], InternalBridge), 0.0);
        // A thread no deeper than a layer does not rule it out.
        let shallow = OverInfill {
            bridged_depth: 0,
            ..over(NoFilter)
        };
        let found =
            bridge_over_infill_by(std::slice::from_ref(&stack), &beneath, &shallow).unwrap();
        assert!(area(&found[0][2], InternalBridge) > 30.0);
    }

    #[test]
    fn every_filament_s_bridges_are_sorted_out_by_the_whole_layer_beneath() {
        // Two filaments side by side; the second's solid strip lies over
        // the first's sparse fill on the layer below.
        let (left, right) = (block(0.0, 0.0, 15.0, 30.0), block(15.0, 0.0, 30.0, 30.0));
        let whole = block(0.0, 0.0, 30.0, 30.0);
        let first = vec![vec![Surface::new(Sparse, whole.clone())], Vec::new()];
        let strip = block(5.0, 0.0, 7.0, 30.0);
        let second = vec![
            Vec::new(),
            vec![
                Surface::new(InternalSolid, strip.clone()),
                Surface::new(Sparse, right.clone()),
            ],
        ];
        let _ = left;
        let beneath = [
            Beneath::gather([(first[0].as_slice(), std::slice::from_ref(&whole))]),
            Beneath::default(),
        ];
        let found =
            bridge_over_infill_by(&[first.clone(), second], &beneath, &over(Limited)).unwrap();
        assert_eq!(found[0], first);
        assert!((area(&found[1][1], InternalBridge) - 60.0).abs() < 1e-3);
        assert!((area(&found[1][1], Sparse) - 450.0).abs() < 1e-3);
    }
}
