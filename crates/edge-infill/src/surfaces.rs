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
    /// The direction its lines are to run, in radians counter-clockwise
    /// from +x, where the surface has one of its own: a bridge runs from
    /// one support to the other. None leaves it to the fill.
    pub angle: Option<f64>,
}

impl Surface {
    fn new(kind: SurfaceKind, area: ExPolygon) -> Self {
        Self {
            kind,
            area,
            angle: None,
        }
    }
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
    /// The spacing of solid lines when the walls are to be kept as thick
    /// measured straight up as they are sideways
    /// (`ensure_vertical_shell_thickness` at `ensure_all`): whatever of a
    /// layer's fill is not over and under the fill of the layers within
    /// a shell of it is then made solid.
    pub ensure: Option<Coord>,
}

/// The one set of walls a layer would have were it all in one filament.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallsAround {
    /// From the outline to where the fill would start.
    pub depth: Coord,
    /// The least distance between two of its walls.
    pub spacing: Coord,
}

/// Each layer's area inside the walls taken over every filament:
/// `by_filament[f][i]` is filament `f`'s on layer `i` and `slices[i]` the
/// layer's whole cross-section as printed. Where filaments meet inside a
/// part each has walls of its own; with `walls` given, the area inside
/// one set of walls around the whole layer counts as well, so that those
/// inner walls are not taken for the part's shell. This is the `holes` of
/// the reference's `PrintObject::discover_vertical_shells`.
pub fn inside_walls(
    by_filament: &[Vec<&[ExPolygon]>],
    slices: &[&[ExPolygon]],
    walls: Option<WallsAround>,
) -> Result<Vec<Vec<ExPolygon>>, ClipError> {
    (0..slices.len())
        .into_par_iter()
        .map(|layer| {
            let mut all: Vec<ExPolygon> = by_filament
                .iter()
                .flat_map(|own| own[layer].iter().cloned())
                .collect();
            if let Some(walls) = walls {
                // The layer's pieces are made to merge by growing them first.
                let merge = (0.3 * walls.spacing as f64) as Coord;
                all.extend(edge_clip::offset2(
                    slices[layer],
                    merge,
                    -walls.depth - merge,
                    Join::MITER,
                )?);
            }
            if by_filament.len() > 1 || walls.is_some() {
                edge_clip::union_ex(&all)
            } else {
                Ok(all)
            }
        })
        .collect()
}

/// Splits each layer's infill area by what it is. `slices[i]` is layer
/// `i`'s cross-section as printed, `outlines[i]` its cross-section as
/// modelled, `infill[i]` the part of the slice left inside the walls, and
/// `inside[i]` the layer's area inside the walls over every filament
/// (`inside_walls`), which is read only when `shells.ensure` is set.
/// The two cross-sections differ only where a layer was shrunk for the
/// elephant's foot: a layer is compared with its neighbours as modelled, so
/// the shrinking does not make the layer above look like an overhang. A layer's surfaces do not overlap and together cover its
/// infill area, except that sparse areas are left out at zero density.
pub fn classify(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    infill: &[&[ExPolygon]],
    inside: &[Vec<ExPolygon>],
    shells: &Shells,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(slices.len(), infill.len(), "one infill area per layer");
    if shells.ensure.is_some() {
        assert_eq!(slices.len(), inside.len(), "one area inside per layer");
    }
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
                surfaces.extend(areas.into_iter().map(|area| Surface::new(kind, area)));
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
                .clone()
                .flat_map(|other| &faces[other].0)
                .chain(below.clone().flat_map(|other| &faces[other].1))
                .cloned()
                .collect();

            let (mut solid, mut sparse) = if shells.sparse_density >= 1.0 {
                (internal, Vec::new())
            } else if let Some(spacing) = shells.ensure {
                // What the fill of every layer within a shell of this
                // one covers; the rest of this layer's fill lies under or
                // over their walls and is part of the shell.
                let mut covered = inside[layer].clone();
                for other in above.chain(below) {
                    if covered.is_empty() {
                        break;
                    }
                    covered = edge_clip::intersection(&covered, &inside[other])?;
                }
                let mut kept = if shell.is_empty() {
                    Vec::new()
                } else {
                    edge_clip::intersection(&internal, &shell)?
                };
                kept.extend(edge_clip::difference(&internal, &covered)?);
                if kept.is_empty() {
                    (Vec::new(), internal)
                } else {
                    let object = match (layer.checked_sub(1), outlines.get(layer + 1)) {
                        (Some(lower), Some(upper)) => {
                            edge_clip::intersection(outlines[lower], upper)?
                        }
                        _ => Vec::new(),
                    };
                    let regular = regularized(&kept, &internal, &object, spacing)?;
                    (
                        edge_clip::intersection(&internal, &regular)?,
                        edge_clip::difference(&internal, &regular)?,
                    )
                }
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

/// A shell made fit to fill with lines `spacing` apart, as the reference
/// does in `discover_vertical_shells`: strips too narrow for a line are
/// dropped, gaps too narrow for sparse fill are closed, and scattered
/// drops that stand for nothing are removed. `internal` is the fill the
/// shell is part of and `object` what the part has both above and below.
fn regularized(
    shell: &[ExPolygon],
    internal: &[ExPolygon],
    object: &[ExPolygon],
    spacing: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    // A little more than a line, as the reference takes it.
    let line = 1.05 * spacing as f64;
    let narrow = (0.5 * 0.65 * line) as Coord;
    let gap = (0.5 * 1.2 * line) as Coord;
    let overlap = (0.2 * line) as Coord;
    let join = Join::Square;
    let grown = edge_clip::offset2(&edge_clip::union_ex(shell)?, -narrow, narrow + gap, join)?;
    let regular = edge_clip::offset(&grown, overlap - gap, join)?;
    let line_mm = edge_geom::unscaled(line as Coord);
    if regular.iter().all(|drop| drop.area_mm2() >= line_mm * 8.0) {
        return Ok(regular);
    }
    let volume = edge_clip::closing(internal, 100, Join::MITER)?;
    let pieces = |regions: &[ExPolygon]| regions.iter().map(|r| 1 + r.holes.len()).sum::<usize>();
    let mut kept = Vec::with_capacity(regular.len());
    for drop in regular {
        let area = drop.area_mm2();
        let own = std::slice::from_ref(&drop);
        let slight = area < line_mm * 1.5
            || (area < line_mm * 8.0 && edge_clip::difference(own, object)?.is_empty());
        // A drop that takes a whole piece of the fill with it stays.
        let scattered = slight && {
            let around = edge_clip::offset(own, line as Coord, Join::MITER)?;
            pieces(&edge_clip::difference(&volume, &around)?) >= pieces(&volume)
        };
        if !scattered {
            kept.push(drop);
        }
    }
    Ok(kept)
}

/// How far the surfaces that face outward reach into the fill beside
/// them, so that they have something to hold on to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchoring {
    /// How far into sparse fill: the reference's one wall spacing.
    pub into_sparse: Coord,
    /// How far into solid fill, and for a bridge into a top surface: the
    /// reference's width of the walls times the square root of two.
    pub into_solid: Coord,
    /// Gaps narrower than twice this left in a grown surface are closed.
    pub closing: Coord,
    /// What is left of a sparse area is made solid when smaller than
    /// this many mm².
    pub minimum_sparse_area: f64,
}

/// Grows each layer's bridges, bottoms and tops into the fill beside
/// them, and gives every bridge the direction that carries it from
/// support to support (`LayerRegion::process_external_surfaces`). The
/// reference grows in waves that cannot leave the area they start in;
/// here a surface is grown by the same distance and cut to the fill it
/// may grow into.
pub fn anchor(
    surfaces: &[Vec<Surface>],
    anchoring: &Anchoring,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    let join = Join::MITER;
    surfaces
        .par_iter()
        .map(|layer| {
            let of = |kind: SurfaceKind| -> Vec<ExPolygon> {
                layer
                    .iter()
                    .filter(|s| s.kind == kind)
                    .map(|s| s.area.clone())
                    .collect()
            };
            let (bridges, bottoms) = (of(SurfaceKind::Bridge), of(SurfaceKind::Bottom));
            let mut top = of(SurfaceKind::Top);
            if bridges.is_empty() && bottoms.is_empty() && top.is_empty() {
                return Ok(layer.clone());
            }
            let mut solid = of(SurfaceKind::InternalSolid);
            let mut sparse = of(SurfaceKind::Sparse);
            let mut out = Vec::with_capacity(layer.len());

            // A surface with what it gains from each kind of fill beside
            // it, closed up and kept to what it started from and gained.
            let grown = |surface: &[ExPolygon],
                         into: &[(&[ExPolygon], Coord)]|
             -> Result<Vec<ExPolygon>, ClipError> {
                let mut all = surface.to_vec();
                let mut allowed = surface.to_vec();
                for (zone, reach) in into {
                    if zone.is_empty() || *reach <= 0 {
                        continue;
                    }
                    all.extend(edge_clip::intersection(
                        &edge_clip::offset(surface, *reach, join)?,
                        zone,
                    )?);
                    allowed.extend(zone.iter().cloned());
                }
                if all.len() == surface.len() {
                    return edge_clip::union_ex(surface);
                }
                edge_clip::intersection(
                    &edge_clip::closing(&all, anchoring.closing, join)?,
                    &edge_clip::union_ex(&allowed)?,
                )
            };

            // Bridges first, each with its own direction, judged by the
            // edges it has no fill beside.
            if !bridges.is_empty() {
                let beside: Vec<ExPolygon> =
                    solid.iter().chain(&sparse).chain(&top).cloned().collect();
                let beside = edge_clip::offset(&beside, BESIDE, join)?;
                let mut taken: Vec<ExPolygon> = Vec::new();
                for bridge in &bridges {
                    let angle = bridge_direction(bridge, &beside);
                    let own = std::slice::from_ref(bridge);
                    let mut area = grown(
                        own,
                        &[
                            (&solid, anchoring.into_solid),
                            (&sparse, anchoring.into_sparse),
                            (&top, anchoring.into_solid),
                        ],
                    )?;
                    if !taken.is_empty() {
                        area = edge_clip::difference(&area, &taken)?;
                    }
                    solid = edge_clip::difference(&solid, &area)?;
                    sparse = edge_clip::difference(&sparse, &area)?;
                    top = edge_clip::difference(&top, &area)?;
                    taken.extend(area.iter().cloned());
                    out.extend(area.into_iter().map(|area| Surface {
                        kind: SurfaceKind::Bridge,
                        area,
                        angle: Some(angle),
                    }));
                }
            }
            // Then what rests on the bed, and last the tops.
            let mut spread = |kind: SurfaceKind,
                              surface: &[ExPolygon],
                              solid: &mut Vec<ExPolygon>,
                              sparse: &mut Vec<ExPolygon>|
             -> Result<(), ClipError> {
                if surface.is_empty() {
                    return Ok(());
                }
                let area = grown(
                    surface,
                    &[
                        (solid, anchoring.into_solid),
                        (sparse, anchoring.into_sparse),
                    ],
                )?;
                *solid = edge_clip::difference(solid, &area)?;
                *sparse = edge_clip::difference(sparse, &area)?;
                out.extend(area.into_iter().map(|area| Surface::new(kind, area)));
                Ok(())
            };
            spread(SurfaceKind::Bottom, &bottoms, &mut solid, &mut sparse)?;
            spread(SurfaceKind::Top, &top, &mut solid, &mut sparse)?;

            // What is left of the sparse fill may be too small to be
            // worth a pattern.
            let (small, large): (Vec<ExPolygon>, Vec<ExPolygon>) = sparse
                .into_iter()
                .partition(|area| area.area_mm2() < anchoring.minimum_sparse_area);
            if !small.is_empty() && !of(SurfaceKind::Sparse).is_empty() {
                solid.extend(small);
                solid = edge_clip::union_ex(&solid)?;
            }
            out.extend(
                solid
                    .into_iter()
                    .map(|area| Surface::new(SurfaceKind::InternalSolid, area)),
            );
            out.extend(
                layer
                    .iter()
                    .filter(|s| s.kind == SurfaceKind::InternalBridge)
                    .cloned(),
            );
            out.extend(
                large
                    .into_iter()
                    .map(|area| Surface::new(SurfaceKind::Sparse, area)),
            );
            Ok(out)
        })
        .collect()
}

/// How far fill is grown before a bridge's edges are tested against it,
/// so that an edge along it counts as beside it.
const BESIDE: Coord = 2_000;

/// Whether a point, in mm, lies in any of the regions.
fn contains(regions: &[ExPolygon], x: f64, y: f64) -> bool {
    regions.iter().any(|region| {
        let mut inside = false;
        for ring in region.rings() {
            let points = &ring.points;
            for (index, a) in points.iter().enumerate() {
                let b = points[(index + 1) % points.len()];
                let (ax, ay) = (edge_geom::unscaled(a.x), edge_geom::unscaled(a.y));
                let (bx, by) = (edge_geom::unscaled(b.x), edge_geom::unscaled(b.y));
                if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                    inside = !inside;
                }
            }
        }
        inside
    })
}

/// The direction a bridge's lines should run, in radians from 0 up to π
/// (BridgeDetector.hpp `detect_bridging_direction`). `beside` is the
/// fill around it, which holds a line's end up. Lines run along the
/// edges that have no fill beside them, so that as little line as can be
/// ends in the air; a bridge with fill all round it is crossed the short
/// way.
fn bridge_direction(bridge: &ExPolygon, beside: &[ExPolygon]) -> f64 {
    // The edges with nothing beside them, in pieces of a millimetre or
    // less so that an edge partly held is partly counted.
    let mut floating: Vec<(f64, f64)> = Vec::new();
    for ring in bridge.rings() {
        let points = &ring.points;
        for (index, a) in points.iter().enumerate() {
            let b = points[(index + 1) % points.len()];
            let (ax, ay) = (edge_geom::unscaled(a.x), edge_geom::unscaled(a.y));
            let (dx, dy) = (edge_geom::unscaled(b.x) - ax, edge_geom::unscaled(b.y) - ay);
            let pieces = dx.hypot(dy).ceil().max(1.0) as usize;
            for piece in 0..pieces {
                let middle = (piece as f64 + 0.5) / pieces as f64;
                if !contains(beside, ax + dx * middle, ay + dy * middle) {
                    floating.push((dx / pieces as f64, dy / pieces as f64));
                }
            }
        }
    }
    let direction = if floating.is_empty() {
        short_way(bridge)
    } else {
        // Each edge's own direction is tried; the one the edges together
        // run along most wins. Tried in a fixed order, so the choice
        // between equals is always the same.
        let mut tried: std::collections::BTreeMap<i64, (f64, f64)> =
            std::collections::BTreeMap::new();
        for (dx, dy) in &floating {
            let length = dx.hypot(*dy);
            if length > 0.0 {
                let normal = (-dy / length, dx / length);
                let key = (normal.1.atan2(normal.0) * 1000.0).ceil() as i64;
                tried.entry(key).or_insert(normal);
            }
        }
        let mut best = ((1.0, 0.0), f64::MAX);
        for normal in tried.values() {
            let across: f64 = floating
                .iter()
                .map(|(dx, dy)| (dx * normal.0 + dy * normal.1).abs())
                .sum();
            if across < best.1 {
                best = ((normal.1, -normal.0), across);
            }
        }
        best.0
    };
    direction
        .1
        .atan2(direction.0)
        .rem_euclid(std::f64::consts::PI)
}

/// The direction across the narrower way of a region: the lesser of the
/// two axes its area is spread along (PrincipalComponents2D.cpp).
fn short_way(region: &ExPolygon) -> (f64, f64) {
    let Some(origin) = region.contour.points.first().copied() else {
        return (1.0, 0.0);
    };
    let at = |point: &edge_geom::Point2| {
        (
            edge_geom::unscaled(point.x - origin.x),
            edge_geom::unscaled(point.y - origin.y),
        )
    };
    // Area and its first and second moments, ring by ring as fans of
    // triangles; a hole's triangles count against the contour's.
    let (mut area, mut mx, mut my, mut mxx, mut myy, mut mxy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for ring in region.rings() {
        let points = &ring.points;
        if points.len() < 3 {
            continue;
        }
        let (x0, y0) = at(&points[0]);
        for pair in points[1..].windows(2) {
            let ((x1, y1), (x2, y2)) = (at(&pair[0]), at(&pair[1]));
            let a = ((x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0)) / 2.0;
            area += a;
            mx += a * (x0 + x1 + x2) / 3.0;
            my += a * (y0 + y1 + y2) / 3.0;
            mxx += a / 6.0 * (x0 * x0 + x1 * x1 + x2 * x2 + x0 * x1 + x1 * x2 + x0 * x2);
            myy += a / 6.0 * (y0 * y0 + y1 * y1 + y2 * y2 + y0 * y1 + y1 * y2 + y0 * y2);
            mxy += a / 12.0
                * (x0 * (2.0 * y0 + y1 + y2)
                    + x1 * (y0 + 2.0 * y1 + y2)
                    + x2 * (y0 + y1 + 2.0 * y2));
        }
    }
    if area <= 0.0 {
        return (1.0, 0.0);
    }
    let (cx, cy) = (mx / area, my / area);
    let (var_x, var_y, cov) = (
        mxx / area - cx * cx,
        myy / area - cy * cy,
        mxy / area - cx * cy,
    );
    if cov.abs() < 1e-9 {
        return if var_y > var_x {
            (1.0, 0.0)
        } else {
            (0.0, 1.0)
        };
    }
    // The lesser eigenvalue of the spread, and its direction.
    let lesser = 0.5 * (var_x + var_y - ((var_x - var_y).powi(2) + 4.0 * cov * cov).sqrt());
    let (x, y) = ((lesser - var_y) / cov, 1.0);
    let length = x.hypot(y);
    (x / length, y / length)
}

/// What a layer gives the one above it to rest on, taken over every
/// filament printed in it: a filament that starts part of the way up
/// rests on whatever is beneath it, not on its own layer below.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Beneath {
    /// The layer's whole area inside the walls.
    pub infill: Vec<ExPolygon>,
    /// The part of it filled solid: everything but sparse fill.
    pub solid: Vec<ExPolygon>,
}

impl Beneath {
    /// Gathers a layer from each filament's surfaces on it and each
    /// filament's area inside the walls.
    pub fn gather<'a>(parts: impl IntoIterator<Item = (&'a [Surface], &'a [ExPolygon])>) -> Self {
        let mut all = Self::default();
        for (surfaces, infill) in parts {
            all.infill.extend(infill.iter().cloned());
            all.solid.extend(
                surfaces
                    .iter()
                    .filter(|s| s.kind != SurfaceKind::Sparse)
                    .map(|s| s.area.clone()),
            );
        }
        all
    }
}

/// Turns the parts of each layer's internal solid that lie over sparse
/// fill into internal bridges: the first phase of the reference's
/// `PrintObject::bridge_over_infill`, which picks out what is worth
/// bridging. `beneath[i]` is what layer `i` gives the next to rest on,
/// over all filaments as the reference takes it over all regions, and
/// `spacing` the distance between solid lines. The reference goes on to
/// run each bridge out to the sparse lines that carry it; here a bridge
/// is the area picked out and no more.
pub fn bridge_over_infill(
    surfaces: &[Vec<Surface>],
    beneath: &[Beneath],
    spacing: Coord,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(surfaces.len(), beneath.len(), "one layer beneath per layer");
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
            let below = &beneath[layer - 1];
            let footing = edge_clip::offset(
                &edge_clip::offset(&below.solid, -spacing, join)?,
                4 * spacing,
                join,
            )?;
            let open = edge_clip::offset(
                &edge_clip::closing(&below.infill, SNAP, join)?,
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
                    changed.extend(areas.into_iter().map(|area| Surface::new(kind, area)));
                };
                push(SurfaceKind::InternalBridge, bridged);
                push(SurfaceKind::InternalSolid, rest);
            }
            Ok(changed)
        })
        .collect()
}

/// The band, one line's `spacing` wide, round the inside of every bridge
/// over infill on each layer, taken over all filaments
/// (`surfaces[f][i]` is filament `f`'s layer `i`).
pub fn bridge_edges(
    surfaces: &[Vec<Vec<Surface>>],
    spacing: Coord,
) -> Result<Vec<Vec<ExPolygon>>, ClipError> {
    let count = surfaces.iter().map(Vec::len).max().unwrap_or(0);
    (0..count)
        .into_par_iter()
        .map(|layer| {
            let mut edges = Vec::new();
            for own in surfaces {
                for bridge in own
                    .get(layer)
                    .into_iter()
                    .flatten()
                    .filter(|s| s.kind == SurfaceKind::InternalBridge)
                {
                    let whole = std::slice::from_ref(&bridge.area);
                    edges.extend(edge_clip::difference(
                        whole,
                        &edge_clip::offset(whole, -spacing, Join::MITER)?,
                    )?);
                }
            }
            Ok(edges)
        })
        .collect()
}

/// Makes solid the sparse fill that lies under the edge of a bridge on
/// the layer above, where that edge runs along this layer's walls, so the
/// ends of the bridge's lines have something under them: the last step
/// of the reference's `PrintObject::bridge_over_infill`. `edges[i]` is
/// what `bridge_edges` found on layer `i`.
pub fn carry_bridge_edges(
    surfaces: &[Vec<Surface>],
    edges: &[Vec<ExPolygon>],
    spacing: Coord,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    let join = Join::MITER;
    surfaces
        .par_iter()
        .enumerate()
        .map(|(layer, own)| {
            let above = match edges.get(layer + 1) {
                Some(above) if !above.is_empty() => above,
                _ => return Ok(own.clone()),
            };
            let sparse: Vec<ExPolygon> = own
                .iter()
                .filter(|s| s.kind == SurfaceKind::Sparse)
                .map(|s| s.area.clone())
                .collect();
            if sparse.is_empty() {
                return Ok(own.clone());
            }
            // The band of this layer's fill that lies along its walls.
            let all: Vec<ExPolygon> = own.iter().map(|s| s.area.clone()).collect();
            let all = edge_clip::union_ex(&all)?;
            let beside_walls =
                edge_clip::difference(&all, &edge_clip::offset(&all, -spacing, join)?)?;
            // Widened by a line, so that where walls lean and the two
            // bands only just meet there is still room to print.
            let carrying = edge_clip::intersection(
                &sparse,
                &edge_clip::offset(
                    &edge_clip::intersection(above, &beside_walls)?,
                    spacing,
                    join,
                )?,
            )?;
            if carrying.is_empty() {
                return Ok(own.clone());
            }
            let mut solid: Vec<ExPolygon> = own
                .iter()
                .filter(|s| s.kind == SurfaceKind::InternalSolid)
                .map(|s| s.area.clone())
                .collect();
            solid.extend(carrying.iter().cloned());
            let mut changed: Vec<Surface> = own
                .iter()
                .filter(|s| !matches!(s.kind, SurfaceKind::Sparse | SurfaceKind::InternalSolid))
                .cloned()
                .collect();
            let mut push = |kind, areas: Vec<ExPolygon>| {
                changed.extend(areas.into_iter().map(|area| Surface::new(kind, area)));
            };
            push(SurfaceKind::InternalSolid, edge_clip::union_ex(&solid)?);
            push(
                SurfaceKind::Sparse,
                edge_clip::difference(&sparse, &carrying)?,
            );
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
            ensure: None,
        }
    }

    /// Classifies with the infill area equal to the whole slice, and
    /// reports each layer as `(kind, area in mm²)` sorted by kind order.
    fn kinds(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        classify(&slices, &slices, &slices, &[], shells)
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
                &[],
                &shells(1, 1),
            )
            .unwrap();
            found[1].iter().map(|s| s.kind).collect::<Vec<_>>()
        };
        assert_eq!(kinds(&modelled), [Sparse]);
        // Compared with the shrunken layer, the second would get a bridge ring.
        assert_eq!(kinds(&printed), [Bridge, Sparse]);
    }

    /// A stack whose one side leans in by 0.08 mm a layer: too little for
    /// any layer to show a top face, enough that four layers up the fill
    /// starts a third of a millimetre further in.
    fn leaning(count: usize) -> (Vec<Vec<ExPolygon>>, Vec<Vec<ExPolygon>>) {
        (0..count)
            .map(|layer| {
                let side = 20.0 - 0.08 * layer as f64;
                (
                    block(0.0, 0.0, side, 20.0),
                    block(1.0, 1.0, side - 1.0, 19.0),
                )
            })
            .unzip()
    }

    #[test]
    fn fill_under_the_walls_of_the_layers_above_is_made_solid() {
        let (slices, fills) = leaning(14);
        let slices: Vec<&[ExPolygon]> = slices.iter().map(Vec::as_slice).collect();
        let fills: Vec<&[ExPolygon]> = fills.iter().map(Vec::as_slice).collect();
        let inside = inside_walls(std::slice::from_ref(&fills), &slices, None).unwrap();
        let solid_on = |shells: &Shells, layer: usize| {
            classify(&slices, &slices, &fills, &inside, shells).unwrap()[layer]
                .iter()
                .filter(|s| s.kind == SurfaceKind::InternalSolid)
                .map(|s| s.area.area_mm2())
                .sum::<f64>()
        };
        // Layer 4 is clear of the bottom's three layers and the top's five.
        let plain = shells(5, 3);
        assert_eq!(solid_on(&plain, 4), 0.0);
        let ensuring = Shells {
            ensure: Some(scaled(0.4)),
            ..plain
        };
        // The strip the fill four layers up does not cover, 0.32 mm by
        // 18 mm, and the little it is grown by to meet the fill beside it.
        let strip = solid_on(&ensuring, 4);
        assert!((5.76..8.0).contains(&strip), "{strip}");
        // A layer under the top's shell is solid all over either way.
        assert_eq!(
            solid_on(&ensuring, 10).round(),
            solid_on(&plain, 10).round()
        );
    }

    #[test]
    fn a_strip_too_narrow_for_a_line_is_not_made_solid() {
        // Leaning by a quarter as much, the strip is 0.08 mm wide.
        let layers: Vec<(Vec<ExPolygon>, Vec<ExPolygon>)> = (0..14)
            .map(|layer| {
                let side = 20.0 - 0.02 * layer as f64;
                (
                    block(0.0, 0.0, side, 20.0),
                    block(1.0, 1.0, side - 1.0, 19.0),
                )
            })
            .collect();
        let slices: Vec<&[ExPolygon]> = layers.iter().map(|l| l.0.as_slice()).collect();
        let fills: Vec<&[ExPolygon]> = layers.iter().map(|l| l.1.as_slice()).collect();
        let inside = inside_walls(std::slice::from_ref(&fills), &slices, None).unwrap();
        let ensuring = Shells {
            ensure: Some(scaled(0.4)),
            ..shells(5, 3)
        };
        let found = classify(&slices, &slices, &fills, &inside, &ensuring).unwrap();
        assert!(found[4].iter().all(|s| s.kind == SurfaceKind::Sparse));
    }

    #[test]
    fn walls_between_filaments_are_not_taken_for_the_shell() {
        // Two filaments side by side, each with fill a millimetre inside
        // its own half, so a two millimetre band between them has none.
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let (left, right) = (block(1.0, 1.0, 9.0, 19.0), block(11.0, 1.0, 19.0, 19.0));
        let by_filament = [vec![left.as_slice()], vec![right.as_slice()]];
        let area = |regions: &[ExPolygon]| regions.iter().map(ExPolygon::area_mm2).sum::<f64>();
        let apart = inside_walls(&by_filament, &[whole.as_slice()], None).unwrap();
        assert_eq!(area(&apart[0]).round(), 288.0);
        // One set of walls a millimetre deep around the whole layer has
        // the band inside it.
        let walls = WallsAround {
            depth: scaled(1.0),
            spacing: scaled(0.4),
        };
        let together = inside_walls(&by_filament, &[whole.as_slice()], Some(walls)).unwrap();
        assert_eq!(area(&together[0]).round(), 324.0);
    }

    #[test]
    fn sparse_fill_under_a_bridge_s_edge_along_the_walls_is_made_solid() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let of = |kind| vec![Surface::new(kind, whole[0].clone())];
        // Sparse fill with a bridge over all of it on the next layer.
        let stack = vec![of(SurfaceKind::Sparse), of(SurfaceKind::InternalBridge)];
        let spacing = scaled(0.4);
        let edges = bridge_edges(std::slice::from_ref(&stack), spacing).unwrap();
        assert!(edges[0].is_empty());
        // A band 0.4 mm wide round a 20 mm square.
        let band: f64 = edges[1].iter().map(ExPolygon::area_mm2).sum();
        assert!((band - (400.0 - 19.2 * 19.2)).abs() < 1e-3, "{band}");
        let carried = carry_bridge_edges(&stack, &edges, spacing).unwrap();
        let area = |layer: &[Surface], kind| {
            layer
                .iter()
                .filter(|s| s.kind == kind)
                .map(|s| s.area.area_mm2())
                .sum::<f64>()
        };
        // Under it the band is solid, a line wider than the bridge's own.
        let solid = area(&carried[0], SurfaceKind::InternalSolid);
        assert!((solid - (400.0 - 18.4 * 18.4)).abs() < 1e-3, "{solid}");
        assert!((area(&carried[0], SurfaceKind::Sparse) - 18.4 * 18.4).abs() < 1e-3);
        // The bridge's own layer is left as it was.
        assert_eq!(carried[1], stack[1]);

        // A bridge whose edge is nowhere near the walls below carries
        // on the sparse lines alone.
        let small = block(5.0, 5.0, 15.0, 15.0);
        let stack = vec![
            of(SurfaceKind::Sparse),
            vec![Surface::new(SurfaceKind::InternalBridge, small[0].clone())],
        ];
        let edges = bridge_edges(std::slice::from_ref(&stack), spacing).unwrap();
        assert_eq!(
            carry_bridge_edges(&stack, &edges, spacing).unwrap()[0],
            stack[0]
        );
    }

    #[test]
    fn a_filament_that_starts_on_another_s_sparse_fill_is_bridged_over_it() {
        let whole = block(0.0, 0.0, 20.0, 20.0);
        let solid = |kind| vec![Surface::new(kind, whole[0].clone())];
        // The second filament prints nothing on the first layer.
        let own = [Vec::new(), solid(SurfaceKind::InternalSolid)];
        let kinds = |beneath: &[Beneath]| -> Vec<SurfaceKind> {
            bridge_over_infill(&own, beneath, scaled(0.4)).unwrap()[1]
                .iter()
                .map(|s| s.kind)
                .collect()
        };
        // Going by its own layer below, there is nothing to bridge.
        let alone = [Beneath::default(), Beneath::default()];
        assert_eq!(kinds(&alone), [SurfaceKind::InternalSolid]);
        // Going by the whole layer, it lies on the first filament's
        // sparse fill.
        let first = solid(SurfaceKind::Sparse);
        let under = Beneath::gather([(first.as_slice(), whole.as_slice())]);
        assert!(under.solid.is_empty());
        assert_eq!(
            kinds(&[under, Beneath::default()]),
            [SurfaceKind::InternalBridge]
        );
        // On the first filament's solid fill it is left as it was.
        let first = solid(SurfaceKind::InternalSolid);
        let under = Beneath::gather([(first.as_slice(), whole.as_slice())]);
        assert_eq!(
            kinds(&[under, Beneath::default()]),
            [SurfaceKind::InternalSolid]
        );
    }

    #[test]
    fn a_one_layer_sheet_is_a_bottom() {
        let found = kinds(&[block(0.0, 0.0, 10.0, 10.0)], &shells(4, 3));
        assert_eq!(found, [vec![(Bottom, 100.0)]]);
    }

    /// What each layer gives the next when one filament prints them all.
    fn beneath(surfaces: &[Vec<Surface>], infill: &[&[ExPolygon]]) -> Vec<Beneath> {
        surfaces
            .iter()
            .zip(infill)
            .map(|(surfaces, infill)| Beneath::gather([(surfaces.as_slice(), *infill)]))
            .collect()
    }

    /// Classifies a stack of layers and picks out the bridges over infill,
    /// with solid lines 0.4 mm apart.
    fn bridged(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let surfaces = classify(&slices, &slices, &slices, &[], shells).unwrap();
        bridge_over_infill(&surfaces, &beneath(&surfaces, &slices), scaled(0.4))
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
        let before = classify(
            &layers_alone,
            &layers_alone,
            &layers_alone,
            &[],
            &shells(3, 1),
        )
        .unwrap();
        let area = |surfaces: &[Surface], kinds: &[SurfaceKind]| {
            surfaces
                .iter()
                .filter(|s| kinds.contains(&s.kind))
                .map(|s| s.area.area_mm2())
                .sum::<f64>()
        };
        let after =
            bridge_over_infill(&before, &beneath(&before, &layers_alone), scaled(0.4)).unwrap();
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

    fn anchoring() -> Anchoring {
        Anchoring {
            into_sparse: scaled(0.4),
            into_solid: scaled(1.4),
            closing: scaled(0.15),
            minimum_sparse_area: 15.0,
        }
    }

    /// Classifies a stack of layers and anchors what faces outward.
    fn anchored(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<Surface>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let surfaces = classify(&slices, &slices, &slices, &[], shells).unwrap();
        anchor(&surfaces, &anchoring()).unwrap()
    }

    fn area_of(surfaces: &[Surface], kind: SurfaceKind) -> f64 {
        surfaces
            .iter()
            .filter(|s| s.kind == kind)
            .map(|s| s.area.area_mm2())
            .sum()
    }

    #[test]
    fn a_bridge_runs_from_one_support_to_the_other_and_onto_both() {
        // Two posts 20 mm apart along x, and a deck across them.
        let posts: Vec<ExPolygon> =
            [block(0.0, 0.0, 10.0, 10.0), block(30.0, 0.0, 40.0, 10.0)].concat();
        let mut layers = vec![posts; 3];
        layers.extend(vec![block(0.0, 0.0, 40.0, 10.0); 5]);
        let found = anchored(&layers, &shells(1, 1));
        let deck = &found[3];
        let bridges: Vec<&Surface> = deck.iter().filter(|s| s.kind == Bridge).collect();
        let [bridge] = bridges.as_slice() else {
            panic!("expected one bridge, got {}", bridges.len())
        };
        // Along x: its free edges are the deck's two long sides.
        let angle = bridge.angle.unwrap();
        assert!(
            angle.abs() < 1e-6 || (angle - std::f64::consts::PI).abs() < 1e-6,
            "{angle}"
        );
        // The 20 x 10 mm gap, and 0.4 mm onto the fill over each post.
        let area = bridge.area.area_mm2();
        assert!(
            (area - (200.0 + 2.0 * 0.4 * 10.0)).abs() < 0.5,
            "{area} mm²"
        );
        // What it gained, the fill beside it lost: the layer is still covered once.
        let total: f64 = deck.iter().map(|s| s.area.area_mm2()).sum();
        assert!((total - 400.0).abs() < 1e-3, "{total}");

        // Turned a quarter, the same deck is bridged along y.
        let posts: Vec<ExPolygon> =
            [block(0.0, 0.0, 10.0, 10.0), block(0.0, 30.0, 10.0, 40.0)].concat();
        let mut layers = vec![posts; 3];
        layers.extend(vec![block(0.0, 0.0, 10.0, 40.0); 5]);
        let found = anchored(&layers, &shells(1, 1));
        let bridge = found[3].iter().find(|s| s.kind == Bridge).unwrap();
        assert!((bridge.angle.unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-6);
    }

    #[test]
    fn a_bridge_with_support_all_round_goes_the_short_way() {
        // A plate over a pocket 8 mm wide in x and 14 mm long in y.
        let mut pocket = block(6.0, 3.0, 14.0, 17.0).remove(0).contour;
        pocket.points.reverse();
        let mut ring = block(0.0, 0.0, 20.0, 20.0);
        ring[0].holes.push(pocket);
        let mut layers = vec![ring; 3];
        layers.extend(vec![block(0.0, 0.0, 20.0, 20.0); 5]);
        let found = anchored(&layers, &shells(1, 1));
        let bridge = found[3].iter().find(|s| s.kind == Bridge).unwrap();
        let angle = bridge.angle.unwrap();
        assert!(
            angle.abs() < 1e-6 || (angle - std::f64::consts::PI).abs() < 1e-6,
            "{angle}"
        );
        // It reaches onto the fill round the pocket on every side.
        let area = bridge.area.area_mm2();
        assert!(
            area > 8.0 * 14.0 + 15.0 && area < 8.0 * 14.0 + 20.0,
            "{area} mm²"
        );
    }

    #[test]
    fn tops_and_bottoms_reach_into_the_fill_beside_them() {
        // A 20 mm base carrying a 10 mm tower: the base's top is a ring
        // round the sparse fill under the tower, and gains from it.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 4];
        layers.extend(vec![block(5.0, 5.0, 15.0, 15.0); 4]);
        let found = anchored(&layers, &shells(2, 1));
        let top = area_of(&found[3], Top);
        assert!(
            (top - (300.0 + 4.0 * 10.0 * 0.4 - 4.0 * 0.16)).abs() < 0.5,
            "{top} mm²"
        );
        assert!((area_of(&found[3], Sparse) - (100.0 - 4.0 * 10.0 * 0.4 + 4.0 * 0.16)).abs() < 0.5);
        // A layer with nothing facing outward is left as it was.
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let before = classify(&slices, &slices, &slices, &[], &shells(2, 1)).unwrap();
        assert_eq!(found[1], before[1]);
        // A sparse area left too small to be worth a pattern goes solid.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 4];
        layers.extend(vec![block(8.0, 8.0, 12.0, 12.2); 4]);
        let found = anchored(&layers, &shells(2, 1));
        assert_eq!(area_of(&found[3], Sparse), 0.0);
    }
}
