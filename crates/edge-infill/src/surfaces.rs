use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SurfaceKind {
    /// Nothing above: the visible top of the part.
    Top,
    /// On the bed.
    Bottom,
    /// Nothing below, and not on the bed.
    Bridge,
    /// Inside the part but within the shell under a top or over a bottom.
    InternalSolid,
    /// Solid that has only sparse fill beneath it, and is laid as a bridge
    /// from one of that fill's lines to the next.
    InternalBridge,
    /// Inside the part, filled at the sparse density.
    Sparse,
}

/// One area to fill and what it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Surface {
    pub kind: SurfaceKind,
    pub area: ExPolygon,
}

/// What classification needs to know.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shells {
    /// Solid layers at a top surface, counting the surface itself.
    pub top_layers: usize,
    /// Solid layers at a bottom surface, counting the surface itself.
    pub bottom_layers: usize,
    /// Sparse fill density from 0 to 1. At 1 everything inside is solid.
    pub sparse_density: f64,
    /// Sparse areas smaller than this many mm² are filled solid instead.
    pub minimum_sparse_area: f64,
    /// Exposed strips narrower than twice this are ignored; the reference
    /// uses a tenth of the external wall width.
    pub opening: Coord,
}

/// Splits each layer's infill area by what it is. `slices[i]` is layer
/// `i`'s cross-section as printed, `outlines[i]` its cross-section as
/// modelled, and `infill[i]` the part of the slice left inside the walls.
/// The two cross-sections differ only where a layer was shrunk for the
/// elephant's foot: a layer is compared with its neighbours as modelled, so
/// the shrinking does not make the layer above look like an overhang. A layer's surfaces do not overlap and together cover its
/// infill area, except that sparse areas are left out at zero density.
pub fn classify(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    infill: &[&[ExPolygon]],
    shells: &Shells,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(slices.len(), infill.len(), "one infill area per layer");
    assert_eq!(slices.len(), outlines.len(), "one outline per layer");
    let count = slices.len();

    // Where each layer's cross-section has nothing above it, and nothing
    // below it (PrintObject::detect_surfaces_type). With no shell layers of
    // a kind, that kind of face gets no special treatment at all.
    let exposed = |layer: usize, neighbour: Option<usize>| match neighbour {
        Some(neighbour) => edge_clip::opening(
            &edge_clip::difference(slices[layer], outlines[neighbour])?,
            shells.opening,
            Join::MITER,
        ),
        None => Ok(slices[layer].to_vec()),
    };
    let faces: Vec<(Vec<ExPolygon>, Vec<ExPolygon>)> = (0..count)
        .into_par_iter()
        .map(|layer| {
            let bottom = if shells.bottom_layers == 0 {
                Vec::new()
            } else {
                exposed(layer, layer.checked_sub(1))?
            };
            let mut top = if shells.top_layers == 0 {
                Vec::new()
            } else {
                exposed(layer, Some(layer + 1).filter(|&above| above < count))?
            };
            // A single-layer-thick area is both; it is laid as a bottom.
            if !top.is_empty() && !bottom.is_empty() {
                top = edge_clip::difference(&top, &bottom)?;
            }
            Ok((top, bottom))
        })
        .collect::<Result<_, ClipError>>()?;

    (0..count)
        .into_par_iter()
        .map(|layer| {
            let area = infill[layer];
            let mut surfaces = Vec::new();
            if area.is_empty() {
                return Ok(surfaces);
            }
            let (top, bottom) = &faces[layer];
            let mut push = |kind, areas: Vec<ExPolygon>| {
                surfaces.extend(areas.into_iter().map(|area| Surface { kind, area }));
            };
            // Most layers have nothing exposed; those skip clipping entirely.
            let within = |faces: &[ExPolygon]| {
                if faces.is_empty() {
                    Ok(Vec::new())
                } else {
                    edge_clip::intersection(area, faces)
                }
            };
            push(SurfaceKind::Top, within(top)?);
            let bottom_kind = if layer == 0 {
                SurfaceKind::Bottom
            } else {
                SurfaceKind::Bridge
            };
            push(bottom_kind, within(bottom)?);

            let internal = if top.is_empty() && bottom.is_empty() {
                area.to_vec()
            } else {
                let exposed: Vec<ExPolygon> = top.iter().chain(bottom).cloned().collect();
                edge_clip::difference(area, &exposed)?
            };

            // The shell: what lies under a top face within the next
            // `top_layers - 1` layers up, or over a bottom face within the
            // next `bottom_layers - 1` layers down.
            let above = layer + 1..count.min(layer + shells.top_layers);
            let below = layer.saturating_sub(shells.bottom_layers.saturating_sub(1))..layer;
            let shell: Vec<ExPolygon> = above
                .flat_map(|other| &faces[other].0)
                .chain(below.flat_map(|other| &faces[other].1))
                .cloned()
                .collect();

            let (mut solid, mut sparse) = if shells.sparse_density >= 1.0 {
                (internal, Vec::new())
            } else if shell.is_empty() {
                (Vec::new(), internal)
            } else {
                (
                    edge_clip::intersection(&internal, &shell)?,
                    edge_clip::difference(&internal, &shell)?,
                )
            };
            if shells.sparse_density > 0.0 {
                // Too small to be worth a sparse pattern.
                let (small, large) = sparse
                    .into_iter()
                    .partition(|area| area.area_mm2() < shells.minimum_sparse_area);
                solid.extend::<Vec<ExPolygon>>(small);
                sparse = large;
            } else {
                sparse.clear();
            }
            push(SurfaceKind::InternalSolid, solid);
            push(SurfaceKind::Sparse, sparse);
            Ok(surfaces)
        })
        .collect()
}

/// Turns the parts of each layer's internal solid that lie over sparse
/// fill into internal bridges: the first phase of the reference's
/// `PrintObject::bridge_over_infill`, which picks out what is worth
/// bridging. `infill[i]` is layer `i`'s whole area inside the walls and
/// `spacing` the distance between solid lines. The reference goes on to
/// run each bridge out to the sparse lines that carry it; here a bridge
/// is the area picked out and no more.
pub fn bridge_over_infill(
    surfaces: &[Vec<Surface>],
    infill: &[&[ExPolygon]],
    spacing: Coord,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(surfaces.len(), infill.len(), "one infill area per layer");
    // Edges meant to coincide are made to (the reference's `SCALED_EPSILON`).
    const SNAP: Coord = 100;
    let join = Join::MITER;
    let area_of = |regions: &[ExPolygon]| regions.iter().map(ExPolygon::area_mm2).sum::<f64>();
    let unit = edge_geom::unscaled(spacing);
    (0..surfaces.len())
        .into_par_iter()
        .map(|layer| {
            let own = &surfaces[layer];
            if layer == 0 || !own.iter().any(|s| s.kind == SurfaceKind::InternalSolid) {
                return Ok(own.clone());
            }
            // What the layer beneath gives no solid footing: its area
            // inside the walls, less what is solid there, both first
            // shrunk so that slivers neither count as footing nor are
            // bridged for their own sake.
            let solids: Vec<ExPolygon> = surfaces[layer - 1]
                .iter()
                .filter(|s| s.kind != SurfaceKind::Sparse)
                .map(|s| s.area.clone())
                .collect();
            let footing = edge_clip::offset(
                &edge_clip::offset(&solids, -spacing, join)?,
                4 * spacing,
                join,
            )?;
            let open = edge_clip::offset(
                &edge_clip::closing(infill[layer - 1], SNAP, join)?,
                -3 * spacing,
                join,
            )?;
            let unsupported = edge_clip::difference(&open, &footing)?;
            if unsupported.is_empty() {
                return Ok(own.clone());
            }
            let mut changed = Vec::with_capacity(own.len());
            for surface in own {
                if surface.kind != SurfaceKind::InternalSolid {
                    changed.push(surface.clone());
                    continue;
                }
                let whole = std::slice::from_ref(&surface.area);
                let over = edge_clip::intersection(whole, &unsupported)?;
                let partly = area_of(&over) < surface.area.area_mm2() - 1e-4;
                if over.is_empty() || (partly && area_of(&over) <= 9.0 * unit * unit) {
                    changed.push(surface.clone());
                    continue;
                }
                let mut bridged =
                    edge_clip::intersection(whole, &edge_clip::offset(&over, 4 * spacing, join)?)?;
                // Scraps of solid the bridge would leave beside it join it.
                let beside =
                    edge_clip::difference(whole, &edge_clip::offset(&bridged, spacing, join)?)?;
                bridged.extend(beside.into_iter().filter(|scrap| {
                    let area = scrap.area_mm2();
                    area > unit * unit && area < unit * 12.0
                }));
                let bridged =
                    edge_clip::intersection(&edge_clip::closing(&bridged, SNAP, join)?, whole)?;
                let rest = edge_clip::difference(whole, &bridged)?;
                let mut push = |kind, areas: Vec<ExPolygon>| {
                    changed.extend(areas.into_iter().map(|area| Surface { kind, area }));
                };
                push(SurfaceKind::InternalBridge, bridged);
                push(SurfaceKind::InternalSolid, rest);
            }
            Ok(changed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use edge_geom::{Point2, Polygon, scaled};

    use super::*;

    fn block(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<ExPolygon> {
        let points = [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec();
        vec![ExPolygon {
            contour: Polygon { points },
            holes: Vec::new(),
        }]
    }

    fn shells(top_layers: usize, bottom_layers: usize) -> Shells {
        Shells {
            top_layers,
            bottom_layers,
            sparse_density: 0.2,
            minimum_sparse_area: 15.0,
            opening: scaled(0.045),
        }
    }

    /// Classifies with the infill area equal to the whole slice, and
    /// reports each layer as `(kind, area in mm²)` sorted by kind order.
    fn kinds(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        classify(&slices, &slices, &slices, shells)
            .unwrap()
            .into_iter()
            .map(|surfaces| {
                surfaces
                    .into_iter()
                    .map(|s| (s.kind, s.area.area_mm2().round()))
                    .collect()
            })
            .collect()
    }

    use SurfaceKind::*;

    #[test]
    fn a_plain_block_has_shells_at_both_ends_and_sparse_between() {
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 8];
        let found = kinds(&layers, &shells(3, 2));
        let expected = [
            Bottom,
            InternalSolid,
            Sparse,
            Sparse,
            Sparse,
            InternalSolid,
            InternalSolid,
            Top,
        ];
        assert_eq!(found, expected.map(|kind| vec![(kind, 100.0)]));
    }

    #[test]
    fn a_step_exposes_only_the_ledge() {
        // A 20 mm base four layers tall carrying a 10 mm tower.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 4];
        layers.extend(vec![block(0.0, 0.0, 10.0, 10.0); 4]);
        let found = kinds(&layers, &shells(2, 1));
        // The base's last layer is top on the ledge and sparse under the tower.
        assert_eq!(found[3], [(Top, 300.0), (Sparse, 100.0)]);
        // One layer down the ledge is backed by solid.
        assert_eq!(found[2], [(InternalSolid, 300.0), (Sparse, 100.0)]);
        assert_eq!(found[1], [(Sparse, 400.0)]);
        assert_eq!(found[0], [(Bottom, 400.0)]);
        assert_eq!(found[7], [(Top, 100.0)]);
    }

    #[test]
    fn an_overhang_is_a_bridge_with_solid_above_it() {
        // A 10 mm post that widens to 20 mm at layer 3.
        let mut layers = vec![block(0.0, 0.0, 10.0, 10.0); 3];
        layers.extend(vec![block(0.0, 0.0, 20.0, 10.0); 5]);
        let found = kinds(&layers, &shells(1, 3));
        assert_eq!(found[3], [(Bridge, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[4], [(InternalSolid, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[5], [(InternalSolid, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[6], [(Sparse, 200.0)]);
    }

    #[test]
    fn density_and_shell_counts_change_what_is_solid() {
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 5];
        let solid = Shells {
            sparse_density: 1.0,
            ..shells(1, 1)
        };
        assert_eq!(kinds(&layers, &solid)[2], [(InternalSolid, 100.0)]);
        let hollow = Shells {
            sparse_density: 0.0,
            ..shells(1, 1)
        };
        assert!(kinds(&layers, &hollow)[2].is_empty());
        // No shell layers: the faces are not treated as surfaces at all.
        let open = kinds(&layers, &shells(0, 0));
        assert_eq!(
            (&open[0], &open[4]),
            (&vec![(Sparse, 100.0)], &vec![(Sparse, 100.0)])
        );
        // A sparse pocket under 15 mm² is filled solid.
        let small = vec![block(0.0, 0.0, 3.0, 3.0); 5];
        assert_eq!(kinds(&small, &shells(1, 1))[2], [(InternalSolid, 9.0)]);
    }

    #[test]
    fn neighbours_are_compared_as_modelled_not_as_printed() {
        // The first layer is printed 0.15 mm smaller all round than modelled.
        let modelled = vec![block(0.0, 0.0, 10.0, 10.0); 4];
        let mut printed = modelled.clone();
        printed[0] = block(0.15, 0.15, 9.85, 9.85);
        fn refs(layers: &[Vec<ExPolygon>]) -> Vec<&[ExPolygon]> {
            layers.iter().map(Vec::as_slice).collect()
        }
        let kinds = |outlines: &[Vec<ExPolygon>]| {
            let found = classify(
                &refs(&printed),
                &refs(outlines),
                &refs(&printed),
                &shells(1, 1),
            )
            .unwrap();
            found[1].iter().map(|s| s.kind).collect::<Vec<_>>()
        };
        assert_eq!(kinds(&modelled), [Sparse]);
        // Compared with the shrunken layer, the second would get a bridge ring.
        assert_eq!(kinds(&printed), [Bridge, Sparse]);
    }

    #[test]
    fn a_one_layer_sheet_is_a_bottom() {
        let found = kinds(&[block(0.0, 0.0, 10.0, 10.0)], &shells(4, 3));
        assert_eq!(found, [vec![(Bottom, 100.0)]]);
    }

    /// Classifies a stack of layers and picks out the bridges over infill,
    /// with solid lines 0.4 mm apart.
    fn bridged(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let surfaces = classify(&slices, &slices, &slices, shells).unwrap();
        bridge_over_infill(&surfaces, &slices, scaled(0.4))
            .unwrap()
            .into_iter()
            .map(|surfaces| {
                surfaces
                    .into_iter()
                    .map(|s| (s.kind, s.area.area_mm2().round()))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn the_first_solid_layer_over_sparse_fill_is_a_bridge() {
        // 20 mm square, eight layers: bottom, solid, three sparse, two
        // solid under the top, top.
        let layers = vec![block(0.0, 0.0, 20.0, 20.0); 8];
        let found = bridged(&layers, &shells(3, 2));
        let kinds: Vec<SurfaceKind> = found.iter().map(|layer| layer[0].0).collect();
        assert_eq!(
            kinds,
            [
                Bottom,
                InternalSolid,
                Sparse,
                Sparse,
                Sparse,
                InternalBridge,
                InternalSolid,
                Top
            ]
        );
        // The whole of that layer: nothing beneath it is solid.
        assert_eq!(found[5], [(InternalBridge, 400.0)]);
    }

    #[test]
    fn solid_with_solid_beneath_it_is_not_bridged() {
        // A 20 mm base carrying a 10 mm tower: the base's top shell is a
        // ring of solid round sparse fill under the tower. The ring's
        // first layer bridges; the one above it rests on that.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 6];
        layers.extend(vec![block(5.0, 5.0, 15.0, 15.0); 6]);
        let found = bridged(&layers, &shells(3, 1));
        let has = |layer: usize, kind| found[layer].iter().any(|(found, _)| *found == kind);
        assert!(has(2, Sparse) && !has(2, InternalBridge));
        assert!(has(3, InternalBridge) && !has(3, InternalSolid));
        assert!(has(4, InternalSolid) && !has(4, InternalBridge));
        // Bridge and what is left of the solid cover what the solid did.
        let layers_alone: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let before = classify(&layers_alone, &layers_alone, &layers_alone, &shells(3, 1)).unwrap();
        let area = |surfaces: &[Surface], kinds: &[SurfaceKind]| {
            surfaces
                .iter()
                .filter(|s| kinds.contains(&s.kind))
                .map(|s| s.area.area_mm2())
                .sum::<f64>()
        };
        let after = bridge_over_infill(&before, &layers_alone, scaled(0.4)).unwrap();
        for (was, is) in before.iter().zip(&after) {
            let solid = area(was, &[InternalSolid]);
            assert!((area(is, &[InternalSolid, InternalBridge]) - solid).abs() < 1e-6);
            assert_eq!(
                area(was, &[Sparse, Top, Bottom]),
                area(is, &[Sparse, Top, Bottom])
            );
        }
    }

    #[test]
    fn with_everything_solid_nothing_is_bridged() {
        // At full density there is no sparse fill to bridge over.
        let layers = vec![block(0.0, 0.0, 20.0, 20.0); 8];
        let solid = Shells {
            sparse_density: 1.0,
            ..shells(3, 2)
        };
        let found = bridged(&layers, &solid);
        assert!(
            found
                .iter()
                .flatten()
                .all(|(kind, _)| *kind != InternalBridge)
        );
    }
}
