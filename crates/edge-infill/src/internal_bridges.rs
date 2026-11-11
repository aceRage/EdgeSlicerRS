//! Bridges over infill as the reference's options shape them:
//! `dont_filter_internal_bridges`, which says how small a patch of solid
//! fill over sparse fill is still worth bridging, and
//! `enable_extra_bridge_layer`, which lays a second layer of bridge
//! across the first (PrintObject.cpp `PrintObject::bridge_over_infill`).

use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon};
use rayon::prelude::*;

use crate::FillParams;
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

/// Lays a second layer of bridge over every bridge over infill: what of
/// the next layer's solid and sparse fill lies on a layer's bridges
/// becomes a bridge too, laid square to the first (the last part of
/// PrintObject.cpp `PrintObject::bridge_over_infill`, "Create a second
/// internal bridge layer above the first bridge layer").
/// `surfaces[f][i]` is filament `f`'s layer `i`, and a layer's bridges
/// are taken over every filament. `params` are a layer's above the
/// first: bridges, and patches of a second layer, narrower than twice
/// the solid fill's line width are passed over.
///
/// As in the reference, only a first bridge gets a second layer, and
/// where a layer's bridges are not all laid the same way the second
/// layer is laid square to the last of them.
///
/// Not as the reference has it: there, of a surface that gets a patch
/// of second bridge, whatever is not that patch is put through the same
/// sieve, and what is narrower than two lines is dropped from the layer
/// altogether and nothing is printed in its place. That is an error in
/// the reference (it means to drop bridges too small to matter, not
/// fill); here everything that is not the patch stays what it was.
pub fn second_internal_bridges(
    surfaces: &[Vec<Vec<Surface>>],
    params: &FillParams,
) -> Result<Vec<Vec<Vec<Surface>>>, ClipError> {
    let join = Join::MITER;
    let narrow = params.solid.scaled_width();
    let count = surfaces.iter().map(Vec::len).max().unwrap_or(0);
    // Layer by layer, each from the layer beneath as it was given: a
    // second layer is never read as a first.
    let layers = (0..count)
        .into_par_iter()
        .map(|layer| {
            let unchanged = || -> Vec<Vec<Surface>> {
                surfaces
                    .iter()
                    .map(|own| own.get(layer).cloned().unwrap_or_default())
                    .collect()
            };
            if layer == 0 {
                return Ok(unchanged());
            }
            let first: Vec<&Surface> = surfaces
                .iter()
                .flat_map(|own| own.get(layer - 1).into_iter().flatten())
                .filter(|s| s.kind == SurfaceKind::InternalBridge)
                .collect();
            let Some(last) = first.last() else {
                return Ok(unchanged());
            };
            let angle = last
                .angle
                .unwrap_or_else(|| params.internal_bridge_angle(layer - 1))
                + FRAC_PI_2;
            let bridged: Vec<ExPolygon> = first.iter().map(|s| s.area.clone()).collect();
            let bridged = edge_clip::opening(&bridged, narrow, join)?;
            if bridged.is_empty() {
                return Ok(unchanged());
            }
            surfaces
                .iter()
                .map(|own| {
                    let own = own.get(layer).map_or(&[][..], Vec::as_slice);
                    // What is left alone comes first, then what was cut.
                    let mut kept = Vec::with_capacity(own.len());
                    let mut cut = Vec::new();
                    for surface in own {
                        if !matches!(
                            surface.kind,
                            SurfaceKind::Sparse | SurfaceKind::InternalSolid
                        ) {
                            kept.push(surface.clone());
                        }
                    }
                    for surface in own {
                        if !matches!(
                            surface.kind,
                            SurfaceKind::Sparse | SurfaceKind::InternalSolid
                        ) {
                            continue;
                        }
                        let whole = std::slice::from_ref(&surface.area);
                        let over = edge_clip::opening(
                            &edge_clip::intersection(whole, &bridged)?,
                            narrow,
                            join,
                        )?;
                        if over.is_empty() {
                            kept.push(surface.clone());
                            continue;
                        }
                        let rest = edge_clip::difference(whole, &over)?;
                        cut.extend(over.into_iter().map(|area| Surface {
                            kind: SurfaceKind::InternalBridge,
                            area,
                            angle: Some(angle),
                        }));
                        cut.extend(
                            rest.into_iter()
                                .map(|area| Surface::new(surface.kind, area)),
                        );
                    }
                    kept.extend(cut);
                    Ok(kept)
                })
                .collect::<Result<Vec<Vec<Surface>>, ClipError>>()
        })
        .collect::<Result<Vec<Vec<Vec<Surface>>>, ClipError>>()?;
    // From layer by filament back to filament by layer.
    let mut out: Vec<Vec<Vec<Surface>>> = surfaces
        .iter()
        .map(|own| Vec::with_capacity(own.len()))
        .collect();
    for layer in layers {
        for ((own, given), changed) in layer.into_iter().zip(surfaces).zip(&mut out) {
            if changed.len() < given.len() {
                changed.push(own);
            }
        }
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

    /// What laying needs, with solid lines 0.45 mm wide and a grid of
    /// sparse lines at 45°.
    fn params() -> FillParams {
        crate::InfillSettings {
            pattern: crate::Pattern::Grid,
            ..crate::InfillSettings::default()
        }
        .params(0.2, false)
        .unwrap()
    }

    #[test]
    fn a_bridge_over_infill_gets_a_second_layer_laid_across_it() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole.clone())];
        // Sparse, a bridge over it, two layers of solid, the top.
        let stack = vec![
            of(Sparse),
            of(InternalBridge),
            of(InternalSolid),
            of(InternalSolid),
            of(SurfaceKind::Top),
        ];
        let found = second_internal_bridges(std::slice::from_ref(&stack), &params())
            .unwrap()
            .remove(0);
        // The layer over the bridge is bridge too, square to the first.
        let [second] = found[2].as_slice() else {
            panic!("expected one surface, got {}", found[2].len())
        };
        assert_eq!(second.kind, InternalBridge);
        assert!((second.area.area_mm2() - 400.0).abs() < 1e-6);
        let first = params().internal_bridge_angle(1);
        assert!((second.angle.unwrap() - (first + FRAC_PI_2)).abs() < 1e-12);
        // Nothing else changes: a second layer does not get a third.
        for layer in [0, 1, 3, 4] {
            assert_eq!(found[layer], stack[layer], "layer {layer}");
        }
    }

    #[test]
    fn only_what_lies_on_the_bridge_is_laid_again_and_the_rest_stays() {
        // A bridge 10 mm wide under solid fill 10.6 mm wide, sparse fill
        // beside both, and a top surface elsewhere on the upper layer.
        let bridge = block(0.0, 0.0, 10.0, 20.0);
        let lower = vec![
            Surface::new(InternalBridge, bridge.clone()),
            Surface::new(Sparse, block(10.0, 0.0, 30.0, 20.0)),
        ];
        let top = Surface::new(SurfaceKind::Top, block(30.0, 0.0, 40.0, 20.0));
        let upper = vec![
            Surface::new(InternalSolid, block(0.0, 0.0, 10.6, 20.0)),
            top.clone(),
            Surface::new(Sparse, block(10.6, 0.0, 30.0, 20.0)),
        ];
        let found = second_internal_bridges(&[vec![lower.clone(), upper]], &params())
            .unwrap()
            .remove(0);
        assert_eq!(found[0], lower);
        let layer = &found[1];
        assert!((area(layer, InternalBridge) - 200.0).abs() < 1e-6);
        // The 0.6 mm of solid beside the bridge, narrower than two lines,
        // is still solid: the reference drops it and prints nothing there.
        assert!((area(layer, InternalSolid) - 12.0).abs() < 1e-6);
        assert!((area(layer, Sparse) - 388.0).abs() < 1e-6);
        // What was left alone comes first, in its own order, then what
        // was cut.
        assert_eq!(layer[0], top);
        assert_eq!(layer[1].kind, Sparse);
        assert_eq!(
            layer[2..].iter().map(|s| s.kind).collect::<Vec<_>>(),
            [InternalBridge, InternalSolid]
        );
    }

    #[test]
    fn bridges_and_patches_narrower_than_two_lines_get_no_second_layer() {
        // A bridge 0.8 mm wide: shrunk by a line's 0.45 mm it is gone.
        let narrow = vec![
            vec![Surface::new(InternalBridge, block(0.0, 0.0, 0.8, 20.0))],
            vec![Surface::new(InternalSolid, block(0.0, 0.0, 20.0, 20.0))],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&narrow), &params()).unwrap();
        assert_eq!(found[0], narrow);
        // A wide bridge under solid fill that covers 0.8 mm of it: the
        // patch is too narrow, and the solid fill is left whole.
        let grazed = vec![
            vec![Surface::new(InternalBridge, block(0.0, 0.0, 10.0, 20.0))],
            vec![Surface::new(InternalSolid, block(9.2, 0.0, 20.0, 20.0))],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&grazed), &params()).unwrap();
        assert_eq!(found[0], grazed);
    }

    #[test]
    fn a_second_layer_lies_over_every_filament_s_bridges() {
        // The first filament's bridge under the second filament's solid
        // fill; the second filament prints nothing on the lower layer.
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let first = vec![
            vec![Surface::new(InternalBridge, whole.clone())],
            Vec::new(),
        ];
        let second = vec![Vec::new(), vec![Surface::new(InternalSolid, whole.clone())]];
        let found = second_internal_bridges(&[first.clone(), second], &params()).unwrap();
        assert_eq!(found[0], first);
        assert_eq!(found[1][0], Vec::new());
        assert_eq!(found[1][1].len(), 1);
        assert_eq!(found[1][1][0].kind, InternalBridge);
        // Lines one way on even layers and the other on odd: square to
        // the bridge on the odd layer beneath.
        let lines = FillParams {
            pattern: crate::Pattern::Lines,
            ..params()
        };
        let stack = vec![
            Vec::new(),
            vec![Surface::new(InternalBridge, whole.clone())],
            vec![Surface::new(InternalSolid, whole.clone())],
        ];
        let found = second_internal_bridges(std::slice::from_ref(&stack), &lines).unwrap();
        let angle = found[0][2][0].angle.unwrap();
        assert!((angle - (lines.sparse_angle + FRAC_PI_2 + FRAC_PI_2)).abs() < 1e-12);
    }

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
