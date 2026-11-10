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
/// `i`'s whole cross-section and `infill[i]` the part of it left inside
/// the walls. A layer's surfaces do not overlap and together cover its
/// infill area, except that sparse areas are left out at zero density.
pub fn classify(
    slices: &[&[ExPolygon]],
    infill: &[&[ExPolygon]],
    shells: &Shells,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(slices.len(), infill.len(), "one infill area per layer");
    let count = slices.len();

    // Where each layer's cross-section has nothing above it, and nothing
    // below it (PrintObject::detect_surfaces_type). With no shell layers of
    // a kind, that kind of face gets no special treatment at all.
    let exposed = |layer: usize, neighbour: Option<usize>| match neighbour {
        Some(neighbour) => edge_clip::opening(
            &edge_clip::difference(slices[layer], slices[neighbour])?,
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
        classify(&slices, &slices, shells)
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
    fn a_one_layer_sheet_is_a_bottom() {
        let found = kinds(&[block(0.0, 0.0, 10.0, 10.0)], &shells(4, 3));
        assert_eq!(found, [vec![(Bottom, 100.0)]]);
    }
}
