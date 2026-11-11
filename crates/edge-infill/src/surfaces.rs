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
    pub(crate) fn new(kind: SurfaceKind, area: ExPolygon) -> Self {
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
    /// The least a shell under a top surface is to measure, in mm, the
    /// surface's own layer included (`top_shell_thickness`): where
    /// `top_layers` of them come to less, further layers are made solid
    /// until it is reached. Zero leaves the count to say.
    pub top_thickness: f64,
    /// The same over a bottom surface (`bottom_shell_thickness`).
    pub bottom_thickness: f64,
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
    /// Set where a bridge gets a second layer of bridge over it
    /// (`enable_extra_bridge_layer` for bridges over air): the least
    /// half-width of bridge worth a second layer, which the reference
    /// takes to be the width of the walls.
    pub second_bridge: Option<Coord>,
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

/// A layer's top faces and its bottom faces.
pub(crate) type Faces = (Vec<ExPolygon>, Vec<ExPolygon>);

/// Where each layer's cross-section has nothing above it, and nothing
/// below it (PrintObject::detect_surfaces_type). With no shell layers of
/// a kind, that kind of face gets no special treatment at all.
pub(crate) fn faces(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    shells: &Shells,
) -> Result<Vec<Faces>, ClipError> {
    let count = slices.len();
    let exposed = |layer: usize, neighbour: Option<usize>| match neighbour {
        Some(neighbour) => edge_clip::opening(
            &edge_clip::difference(slices[layer], outlines[neighbour])?,
            shells.opening,
            Join::MITER,
        ),
        None => Ok(slices[layer].to_vec()),
    };
    let found = (0..count)
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
        .collect::<Result<Vec<Faces>, ClipError>>()?;
    // With a second layer asked for over bridges, what lies on a bridge
    // is laid as a bridge too.
    match shells.second_bridge {
        Some(narrow) => crate::bridges::second_bridge_layers(slices, found, narrow),
        None => Ok(found),
    }
}

/// The part of each layer's cross-section that is neither a top face nor
/// a bottom one, walls and all: what the reference calls its internal
/// slices, where a move between fill lines needs no retraction
/// (RetractWhenCrossingPerimeters.cpp `travel_inside_internal_regions`).
/// `slices` and `outlines` are as for `classify`.
pub fn internal(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    shells: &Shells,
) -> Result<Vec<Vec<ExPolygon>>, ClipError> {
    assert_eq!(slices.len(), outlines.len(), "one outline per layer");
    let faces = faces(slices, outlines, shells)?;
    slices
        .par_iter()
        .zip(faces)
        .map(|(slices, (top, bottom))| {
            if top.is_empty() && bottom.is_empty() {
                return Ok(slices.to_vec());
            }
            let exposed: Vec<ExPolygon> = top.into_iter().chain(bottom).collect();
            edge_clip::difference(slices, &exposed)
        })
        .collect()
}

/// Splits each layer's infill area by what it is. `slices[i]` is layer
/// `i`'s cross-section as printed, `outlines[i]` its cross-section as
/// modelled, `infill[i]` the part of the slice left inside the walls, and
/// `inside[i]` the layer's area inside the walls over every filament
/// (`inside_walls`), which is read only when `shells.ensure` is set.
/// `heights[i]` is how thick layer `i` is, in mm: a shell is as many
/// layers as its count, and as many more as its thickness takes.
/// The two cross-sections differ only where a layer was shrunk for the
/// elephant's foot: a layer is compared with its neighbours as modelled, so
/// the shrinking does not make the layer above look like an overhang. A layer's surfaces do not overlap and together cover its
/// infill area, except that sparse areas are left out at zero density.
pub fn classify(
    slices: &[&[ExPolygon]],
    outlines: &[&[ExPolygon]],
    infill: &[&[ExPolygon]],
    inside: &[Vec<ExPolygon>],
    heights: &[f64],
    shells: &Shells,
) -> Result<Vec<Vec<Surface>>, ClipError> {
    assert_eq!(slices.len(), infill.len(), "one infill area per layer");
    assert_eq!(slices.len(), heights.len(), "one height per layer");
    if shells.ensure.is_some() {
        assert_eq!(slices.len(), inside.len(), "one area inside per layer");
    }
    assert_eq!(slices.len(), outlines.len(), "one outline per layer");
    let count = slices.len();

    let faces = faces(slices, outlines, shells)?;
    let reach = Reach::new(
        heights,
        (shells.top_layers, shells.bottom_layers),
        (shells.top_thickness, shells.bottom_thickness),
    );

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
            // next `bottom_layers - 1` layers down, and in as many more
            // as the thickness asked of a shell takes.
            let (above, below) = (reach.above(layer), reach.below(layer));
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

/// `EPSILON` (libslic3r.h), which the reference compares heights by.
const SAME: f64 = 1e-4;

/// Which layers' faces each layer is shell to: those within the count of
/// layers a shell has, and those near enough that the layers between
/// come to less than the thickness asked of it. So the reference walks
/// from a layer in `PrintObject::discover_vertical_shells`, and from a
/// face, by the same test, in `discover_horizontal_shells`
/// (PrintObject.cpp): layers are added until the shell is at least as
/// thick as asked, each at the height it has, and with no layers of a
/// kind asked for there is no shell of that kind whatever its thickness.
struct Reach {
    /// For a top shell and a bottom shell: how many layers, and how
    /// many mm at the least.
    layers: (usize, usize),
    thickness: (f64, f64),
    /// The bottom of each layer and the top of the last, in mm.
    levels: Vec<f64>,
}

impl Reach {
    fn new(heights: &[f64], layers: (usize, usize), thickness: (f64, f64)) -> Self {
        let mut levels = Vec::with_capacity(heights.len() + 1);
        let mut level = 0.0;
        levels.push(level);
        for height in heights {
            level += height;
            levels.push(level);
        }
        Self {
            layers,
            thickness,
            levels,
        }
    }

    fn count(&self) -> usize {
        self.levels.len() - 1
    }

    /// The layers over `layer` whose top faces it is shell under: the
    /// next `top_layers - 1`, and every one after whose top is less than
    /// the thickness above this layer's.
    fn above(&self, layer: usize) -> std::ops::Range<usize> {
        let (count, layers, thickness) = (self.count(), self.layers.0, self.thickness.0);
        let mut end = count.min(layer + layers);
        if layers > 0 {
            let top = |index: usize| self.levels[index + 1];
            while end < count && top(end) - top(layer) < thickness - SAME {
                end += 1;
            }
        }
        (layer + 1).min(end)..end
    }

    /// The layers under `layer` whose bottom faces it is shell over: the
    /// next `bottom_layers - 1` down, and every one before whose bottom
    /// is less than the thickness below this layer's.
    fn below(&self, layer: usize) -> std::ops::Range<usize> {
        let (layers, thickness) = (self.layers.1, self.thickness.1);
        let mut start = layer.saturating_sub(layers.saturating_sub(1));
        if layers > 0 {
            let bottom = |index: usize| self.levels[index];
            while start > 0 && bottom(layer) - bottom(start - 1) < thickness - SAME {
                start -= 1;
            }
        }
        start..layer
    }
}

/// How many layers the shell behind a face is, the face's own among
/// them: for a top face on each layer, and for a bottom face on each.
/// `heights[i]` is how thick layer `i` is, `layers` the count asked of a
/// top shell and of a bottom shell, and `thickness` the least each is to
/// measure in mm. A shell stops where the print does, and with no layers
/// asked for there is none. It is the shell [`classify`] makes solid.
pub fn shell_depths(
    heights: &[f64],
    layers: (usize, usize),
    thickness: (f64, f64),
) -> (Vec<usize>, Vec<usize>) {
    let reach = Reach::new(heights, layers, thickness);
    let count = heights.len();
    let mut top = vec![usize::from(layers.0 > 0); count];
    let mut bottom = vec![usize::from(layers.1 > 0); count];
    for layer in 0..count {
        for face in reach.above(layer) {
            top[face] = top[face].max(face - layer + 1);
        }
        for face in reach.below(layer) {
            bottom[face] = bottom[face].max(layer - face + 1);
        }
    }
    (top, bottom)
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
    /// How much further than `into_solid` a bridge reaches into solid
    /// fill and into a top or bottom surface beside it
    /// (`bridge_expansion_extra`). Zero leaves bridges as they were.
    pub bridge_extra: Coord,
    /// Gaps narrower than twice this left in a grown surface are closed.
    pub closing: Coord,
    /// What is left of a sparse area is made solid when smaller than
    /// this many mm².
    pub minimum_sparse_area: f64,
}

/// Grows each layer's bridges, bottoms and tops into the fill beside
/// them, and gives every bridge the direction that carries it from
/// support to support (`LayerRegion::process_external_surfaces`). The
/// reference grows in waves that cannot leave the area they start in, and
/// so does this.
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
            let (bridges, mut bottoms) = (of(SurfaceKind::Bridge), of(SurfaceKind::Bottom));
            let mut top = of(SurfaceKind::Top);
            if bridges.is_empty() && bottoms.is_empty() && top.is_empty() {
                return Ok(layer.clone());
            }
            let mut solid = of(SurfaceKind::InternalSolid);
            let mut sparse = of(SurfaceKind::Sparse);
            let mut out = Vec::with_capacity(layer.len());

            // A surface with what it gains from each kind of fill beside
            // it, closed up and kept to what it started from and gained.
            // It grows as the reference grows it (`expand_merge_surfaces`,
            // LayerRegion.cpp, by `propagate_waves`,
            // Algorithm/RegionExpansion.cpp): a step at a time within each
            // piece of fill it touches, so that it never jumps the walls
            // between it and fill it does not touch.
            let grown = |surface: &[ExPolygon],
                         into: &[(&[ExPolygon], Coord)]|
             -> Result<Vec<ExPolygon>, ClipError> {
                let mut all = surface.to_vec();
                let mut allowed = surface.to_vec();
                for (zone, reach) in into {
                    if zone.is_empty() || *reach <= 0 {
                        continue;
                    }
                    let steps = crate::bridges::Waves::of(*reach);
                    for source in surface {
                        for piece in *zone {
                            all.extend(crate::bridges::wave(source, piece, &steps)?);
                        }
                    }
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
            // edges it has no fill beside, and grown as the reference grows
            // them (`expand_bridges_detect_orientations`, LayerRegion.cpp):
            // into solid fill, sparse fill and a top surface beside them,
            // in waves that keep to the area they grow in, and those that
            // grow into one another made one. With extra reach set
            // (`bridge_expansion_extra`) they reach that much further into
            // solid fill and a top surface, and into a bottom surface
            // beside them as well. Different from the reference: a
            // bridge's direction is found from the edges with no fill
            // beside them (`bridge_direction`), and the gaps closed in a
            // grown bridge stay inside the fill.
            if !bridges.is_empty() {
                let extra = anchoring.bridge_extra > 0;
                let mut beside: Vec<ExPolygon> =
                    solid.iter().chain(&sparse).chain(&top).cloned().collect();
                if extra {
                    beside.extend(bottoms.iter().cloned());
                }
                let beside = edge_clip::offset(&beside, BESIDE, join)?;
                let angles: Vec<f64> = bridges
                    .iter()
                    .map(|bridge| bridge_direction(bridge, &beside))
                    .collect();
                let reach = anchoring.into_solid + anchoring.bridge_extra;
                let mut zones = vec![
                    crate::bridges::Zone {
                        area: &solid,
                        reach,
                        waves: true,
                    },
                    crate::bridges::Zone {
                        area: &sparse,
                        reach: anchoring.into_sparse,
                        waves: true,
                    },
                    crate::bridges::Zone {
                        area: &top,
                        reach,
                        waves: true,
                    },
                ];
                if extra {
                    zones.push(crate::bridges::Zone {
                        area: &bottoms,
                        reach,
                        waves: true,
                    });
                }
                let grown = crate::bridges::expand(&bridges, &angles, &zones, anchoring.closing)?;
                for (area, angle) in grown {
                    solid = edge_clip::difference(&solid, &area)?;
                    sparse = edge_clip::difference(&sparse, &area)?;
                    top = edge_clip::difference(&top, &area)?;
                    if extra {
                        bottoms = edge_clip::difference(&bottoms, &area)?;
                    }
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
pub(crate) const BESIDE: Coord = 2_000;

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
pub(crate) fn bridge_direction(bridge: &ExPolygon, beside: &[ExPolygon]) -> f64 {
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

    /// Layers of 0.2 mm, as many as asked.
    fn flat(count: usize) -> Vec<f64> {
        vec![0.2; count]
    }

    fn shells(top_layers: usize, bottom_layers: usize) -> Shells {
        Shells {
            top_layers,
            bottom_layers,
            top_thickness: 0.0,
            bottom_thickness: 0.0,
            sparse_density: 0.2,
            minimum_sparse_area: 15.0,
            opening: scaled(0.045),
            ensure: None,
            second_bridge: None,
        }
    }

    /// Classifies with the infill area equal to the whole slice, and
    /// reports each layer as `(kind, area in mm²)` sorted by kind order.
    fn kinds(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        classify(&slices, &slices, &slices, &[], &flat(slices.len()), shells)
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
    fn internal_slices_leave_out_the_faces_and_keep_the_walls() {
        // A wide block for two layers with a narrow one on top of it.
        let layers = [
            block(0.0, 0.0, 20.0, 10.0),
            block(0.0, 0.0, 20.0, 10.0),
            block(0.0, 0.0, 10.0, 10.0),
            block(0.0, 0.0, 10.0, 10.0),
        ];
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let area = |layer: &[ExPolygon]| layer.iter().map(ExPolygon::area_mm2).sum::<f64>();
        let found = internal(&slices, &slices, &shells(3, 2)).unwrap();
        // The first layer is all bottom and the last all top; the second
        // is internal under the narrow block and a top face beside it,
        // and the third whole: nothing is taken off for walls.
        assert!(found[0].is_empty());
        assert!((area(&found[1]) - 100.0).abs() < 0.5);
        assert!((area(&found[2]) - 100.0).abs() < 1e-6);
        assert!(found[3].is_empty());
        // With no shells of either kind there are no faces to leave out.
        let bare = internal(&slices, &slices, &shells(0, 0)).unwrap();
        assert!((area(&bare[0]) - 200.0).abs() < 1e-6);
        assert!((area(&bare[3]) - 100.0).abs() < 1e-6);
    }

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
    fn a_second_layer_of_bridge_moves_the_shell_over_it_up_a_layer() {
        // A 10 mm post that widens to 20 mm at layer 3.
        let mut layers = vec![block(0.0, 0.0, 10.0, 10.0); 3];
        layers.extend(vec![block(0.0, 0.0, 20.0, 10.0); 6]);
        let twice = Shells {
            second_bridge: Some(scaled(0.87)),
            ..shells(1, 3)
        };
        let found = kinds(&layers, &twice);
        // The bridge, a bridge again over it, and the two further solid
        // layers a bottom shell of three has, counted from the second.
        assert_eq!(found[3], [(Bridge, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[4], [(Bridge, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[5], [(InternalSolid, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[6], [(InternalSolid, 100.0), (Sparse, 100.0)]);
        assert_eq!(found[7], [(Sparse, 200.0)]);
        // Without it, as it was: one layer of bridge and two of solid.
        let once = kinds(&layers, &shells(1, 3));
        assert_eq!(once[4], [(InternalSolid, 100.0), (Sparse, 100.0)]);
        assert_eq!(once[6], [(Sparse, 200.0)]);
        // What moves between fill lines without retracting sees the
        // second layer as a face too.
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let area = |layer: &[ExPolygon]| layer.iter().map(ExPolygon::area_mm2).sum::<f64>();
        let inside = internal(&slices, &slices, &twice).unwrap();
        assert!((area(&inside[4]) - 100.0).abs() < 1e-6);
        assert!(
            (area(&internal(&slices, &slices, &shells(1, 3)).unwrap()[4]) - 200.0).abs() < 1e-6
        );
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
                &flat(printed.len()),
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
            classify(
                &slices,
                &slices,
                &fills,
                &inside,
                &flat(slices.len()),
                shells,
            )
            .unwrap()[layer]
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
        let found = classify(
            &slices,
            &slices,
            &fills,
            &inside,
            &flat(slices.len()),
            &ensuring,
        )
        .unwrap();
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
    fn a_one_layer_sheet_is_a_bottom() {
        let found = kinds(&[block(0.0, 0.0, 10.0, 10.0)], &shells(4, 3));
        assert_eq!(found, [vec![(Bottom, 100.0)]]);
    }

    /// `kinds` for layers of the heights given.
    fn kinds_at(
        layers: &[Vec<ExPolygon>],
        heights: &[f64],
        shells: &Shells,
    ) -> Vec<Vec<(SurfaceKind, f64)>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        classify(&slices, &slices, &slices, &[], heights, shells)
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

    /// Shells of the counts given that are to measure `top` and `bottom`
    /// mm at the least.
    fn thick(top_layers: usize, bottom_layers: usize, top: f64, bottom: f64) -> Shells {
        Shells {
            top_thickness: top,
            bottom_thickness: bottom,
            ..shells(top_layers, bottom_layers)
        }
    }

    /// The layers with solid shell of `area` mm² on them.
    fn solid(found: &[Vec<(SurfaceKind, f64)>], area: f64) -> Vec<usize> {
        (0..found.len())
            .filter(|layer| found[*layer].contains(&(InternalSolid, area)))
            .collect()
    }

    #[test]
    fn layers_are_added_until_a_top_shell_is_as_thick_as_asked() {
        // Thirty layers of 0.16 mm: the top is layer 29.
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 30];
        let heights = vec![0.16; 30];
        // Six layers, the top among them, and one over the bed.
        let counted = kinds_at(&layers, &heights, &shells(6, 1));
        assert_eq!(solid(&counted, 100.0), [24, 25, 26, 27, 28]);
        assert_eq!(counted[29], [(Top, 100.0)]);
        // Six make 0.96 mm, short of 1 mm: a seventh is made solid.
        let asked = kinds_at(&layers, &heights, &thick(6, 1, 1.0, 0.0));
        assert_eq!(solid(&asked, 100.0), [23, 24, 25, 26, 27, 28]);
        assert_eq!(asked[22], [(Sparse, 100.0)]);
        assert_eq!(asked[29], [(Top, 100.0)]);
        // And as many as it takes: thirteen of 0.16 mm for 2 mm.
        let deep = kinds_at(&layers, &heights, &thick(6, 1, 2.0, 0.0));
        assert_eq!(solid(&deep, 100.0), (17..29).collect::<Vec<_>>());
        // A bottom shell's thickness is nothing to a top shell.
        let other = kinds_at(&layers, &heights, &thick(6, 1, 0.0, 0.1));
        assert_eq!(other, counted);
    }

    #[test]
    fn a_shell_already_as_thick_as_asked_is_as_counted() {
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 30];
        let counted = kinds_at(&layers, &[0.2; 30], &shells(5, 3));
        // Five layers of 0.2 mm are the 1 mm asked, to the last digit or
        // not: heights are compared as the reference compares them.
        for (top, bottom) in [(1.0, 0.6), (1.0, 0.0), (0.3, 0.3), (0.0, 0.0)] {
            let asked = kinds_at(&layers, &[0.2; 30], &thick(5, 3, top, bottom));
            assert_eq!(asked, counted, "{top} and {bottom} mm");
        }
        // 0.96 mm of six layers of 0.16 mm likewise.
        assert_eq!(
            kinds_at(&layers, &[0.16; 30], &thick(6, 1, 0.96, 0.0)),
            kinds_at(&layers, &[0.16; 30], &shells(6, 1))
        );
    }

    #[test]
    fn layers_are_added_until_a_bottom_shell_is_as_thick_as_asked() {
        // A first layer of 0.25 mm and the rest of 0.12 mm.
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 30];
        let mut heights = vec![0.12; 30];
        heights[0] = 0.25;
        let counted = kinds_at(&layers, &heights, &shells(1, 3));
        assert_eq!(solid(&counted, 100.0), [1, 2]);
        // Layer 5 starts at 0.73 mm, under the 0.8 mm asked, and layer 6
        // at 0.85 mm.
        let asked = kinds_at(&layers, &heights, &thick(1, 3, 0.0, 0.8));
        assert_eq!(solid(&asked, 100.0), [1, 2, 3, 4, 5]);
        assert_eq!(asked[0], [(Bottom, 100.0)]);
        assert_eq!(asked[6], [(Sparse, 100.0)]);
    }

    #[test]
    fn a_shell_is_measured_in_the_layers_as_they_lie() {
        // Twenty layers of 0.2 mm, the last five made 0.19 mm.
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 20];
        let mut heights = vec![0.2; 20];
        let asked = thick(5, 1, 1.0, 0.0);
        assert_eq!(
            solid(&kinds_at(&layers, &heights, &asked), 100.0),
            [15, 16, 17, 18]
        );
        heights[15..].fill(0.19);
        // The five now make 0.95 mm: a sixth is made solid under them.
        assert_eq!(
            solid(&kinds_at(&layers, &heights, &asked), 100.0),
            [14, 15, 16, 17, 18]
        );
        // Made thicker, they need no more than they were.
        heights[15..].fill(0.21);
        assert_eq!(
            solid(&kinds_at(&layers, &heights, &asked), 100.0),
            [15, 16, 17, 18]
        );
    }

    #[test]
    fn no_shell_layers_is_no_shell_whatever_its_thickness() {
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 12];
        let heights = [0.16; 12];
        let none = kinds_at(&layers, &heights, &shells(0, 0));
        assert_eq!(kinds_at(&layers, &heights, &thick(0, 0, 1.0, 1.0)), none);
        assert!(solid(&none, 100.0).is_empty());
        // One kind without the other.
        let bottom_only = kinds_at(&layers, &heights, &thick(0, 2, 1.0, 0.0));
        assert_eq!(bottom_only, kinds_at(&layers, &heights, &shells(0, 2)));
        assert_eq!(solid(&bottom_only, 100.0), [1]);
    }

    #[test]
    fn a_ledge_and_an_overhang_get_shells_as_thick_as_asked() {
        // A 20 mm base ten layers tall carrying a 10 mm tower, in layers
        // of 0.16 mm: the ledge is the top of layer 9.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 10];
        layers.extend(vec![block(0.0, 0.0, 10.0, 10.0); 10]);
        let found = kinds_at(&layers, &[0.16; 20], &thick(3, 1, 1.0, 0.0));
        // Three layers by the count; six under the ledge make 0.96 mm.
        assert_eq!(solid(&found, 300.0), [3, 4, 5, 6, 7, 8]);
        assert!(found[9].contains(&(Top, 300.0)));
        assert_eq!(found[2], [(Sparse, 400.0)]);

        // A 10 mm post that widens to 30 mm at layer 4, in layers of
        // 0.12 mm: over the bridge, the layers that start within 0.8 mm
        // of where it does. The shell over the bed is as deep, and ends
        // two layers into the wide part.
        let mut layers = vec![block(0.0, 0.0, 10.0, 10.0); 4];
        layers.extend(vec![block(0.0, 0.0, 30.0, 10.0); 12]);
        let over_bridge = |found: &[Vec<(SurfaceKind, f64)>]| -> Vec<usize> {
            (0..found.len())
                .filter(|layer| {
                    let solid: f64 = found[*layer]
                        .iter()
                        .filter(|(kind, _)| *kind == InternalSolid)
                        .map(|(_, area)| area)
                        .sum();
                    solid >= 200.0
                })
                .collect()
        };
        let found = kinds_at(&layers, &[0.12; 16], &thick(1, 3, 0.0, 0.8));
        assert!(found[4].contains(&(Bridge, 200.0)));
        assert_eq!(over_bridge(&found), [5, 6, 7, 8, 9, 10]);
        // Beside the bridge and below it, the shell over the bed alone.
        assert_eq!(solid(&found, 100.0), [1, 2, 3, 4]);
        let counted = kinds_at(&layers, &[0.12; 16], &shells(1, 3));
        assert_eq!(over_bridge(&counted), [5, 6]);
    }

    #[test]
    fn walls_kept_thick_go_by_the_same_shell() {
        // With `ensure` the fill of every layer within a shell is looked
        // at, and that is as many layers as the thickness takes too.
        let layers = vec![block(0.0, 0.0, 10.0, 10.0); 30];
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let solid_layers = |shells: &Shells| -> Vec<usize> {
            let ensuring = Shells {
                ensure: Some(scaled(0.4)),
                ..*shells
            };
            let found = classify(&slices, &slices, &slices, &layers, &[0.16; 30], &ensuring);
            (0..30)
                .filter(|layer| {
                    found.as_ref().unwrap()[*layer]
                        .iter()
                        .any(|s| s.kind == InternalSolid)
                })
                .collect()
        };
        assert_eq!(solid_layers(&shells(6, 1)), [24, 25, 26, 27, 28]);
        assert_eq!(
            solid_layers(&thick(6, 1, 1.0, 0.0)),
            [23, 24, 25, 26, 27, 28]
        );
    }

    #[test]
    fn the_depth_of_a_shell_behind_a_face() {
        let heights = [0.16; 30];
        let (top, bottom) = shell_depths(&heights, (6, 4), (1.0, 0.0));
        // Seven layers under a top, as far as there are that many.
        assert_eq!(top[29], 7);
        assert_eq!(top[10], 7);
        assert_eq!(top[3], 4);
        assert_eq!(top[0], 1);
        // Four over a bottom, by the count alone.
        assert_eq!(bottom[0], 4);
        assert_eq!(bottom[28], 2);
        let (top, bottom) = shell_depths(&heights, (6, 0), (0.0, 1.0));
        assert_eq!(top[29], 6);
        assert!(bottom.iter().all(|depth| *depth == 0));
    }

    fn anchoring() -> Anchoring {
        Anchoring {
            into_sparse: scaled(0.4),
            into_solid: scaled(1.4),
            bridge_extra: 0,
            closing: scaled(0.15),
            minimum_sparse_area: 15.0,
        }
    }

    /// Classifies a stack of layers and anchors what faces outward.
    fn anchored(layers: &[Vec<ExPolygon>], shells: &Shells) -> Vec<Vec<Surface>> {
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let surfaces =
            classify(&slices, &slices, &slices, &[], &flat(slices.len()), shells).unwrap();
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
    fn a_bridge_reaches_further_onto_solid_fill_by_the_extra_set() {
        // Two posts 20 mm apart and a deck three layers thick across
        // them, all of it within the top's shell: over the posts the
        // deck's first layer is solid fill.
        let posts: Vec<ExPolygon> =
            [block(0.0, 0.0, 10.0, 10.0), block(30.0, 0.0, 40.0, 10.0)].concat();
        let mut layers = vec![posts; 3];
        layers.extend(vec![block(0.0, 0.0, 40.0, 10.0); 3]);
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let surfaces = classify(
            &slices,
            &slices,
            &slices,
            &[],
            &flat(slices.len()),
            &shells(4, 1),
        )
        .unwrap();
        assert_eq!(area_of(&surfaces[3], InternalSolid).round(), 200.0);
        let grown = |extra: f64| {
            let anchoring = Anchoring {
                bridge_extra: scaled(extra),
                ..anchoring()
            };
            anchor(&surfaces, &anchoring).unwrap().swap_remove(3)
        };
        // With none, the gap and 1.4 mm onto the fill over each post.
        let plain = grown(0.0);
        assert!((area_of(&plain, Bridge) - 228.0).abs() < 0.5);
        // With 2 mm, that much further onto each, and the solid fill
        // gives up what the bridge gains.
        let further = grown(2.0);
        let bridges: Vec<&Surface> = further.iter().filter(|s| s.kind == Bridge).collect();
        let [bridge] = bridges.as_slice() else {
            panic!("expected one bridge, got {}", bridges.len())
        };
        assert!(
            (bridge.area.area_mm2() - 268.0).abs() < 0.5,
            "{}",
            bridge.area.area_mm2()
        );
        assert!((area_of(&further, InternalSolid) - 132.0).abs() < 0.5);
        let total: f64 = further.iter().map(|s| s.area.area_mm2()).sum();
        assert!((total - 400.0).abs() < 1e-3, "{total}");
        // Still laid from one post to the other.
        let angle = bridge.angle.unwrap();
        assert!(
            angle.abs() < 1e-6 || (angle - std::f64::consts::PI).abs() < 1e-6,
            "{angle}"
        );
        // With more than the posts are wide the whole layer is bridge.
        let all = grown(50.0);
        assert!((area_of(&all, Bridge) - 400.0).abs() < 1e-3);
        assert_eq!(area_of(&all, InternalSolid), 0.0);
        // Sparse fill beside a bridge is reached no further: the same
        // deck with no shell over the posts.
        let surfaces = classify(
            &slices,
            &slices,
            &slices,
            &[],
            &flat(slices.len()),
            &shells(1, 1),
        )
        .unwrap();
        let onto_sparse = |extra: f64| {
            let anchoring = Anchoring {
                bridge_extra: scaled(extra),
                ..anchoring()
            };
            area_of(&anchor(&surfaces, &anchoring).unwrap()[3], Bridge)
        };
        assert!((onto_sparse(2.0) - onto_sparse(0.0)).abs() < 1e-3);
    }

    #[test]
    fn with_extra_reach_a_bridge_grows_into_a_bottom_beside_it() {
        // A bridge and a bottom surface side by side on one layer.
        let layer = vec![
            Surface::new(Bridge, block(0.0, 0.0, 10.0, 10.0).remove(0)),
            Surface::new(Bottom, block(10.0, 0.0, 20.0, 10.0).remove(0)),
        ];
        let grown = |extra: f64| {
            let anchoring = Anchoring {
                bridge_extra: scaled(extra),
                ..anchoring()
            };
            anchor(std::slice::from_ref(&layer), &anchoring)
                .unwrap()
                .remove(0)
        };
        // With none, each keeps its own.
        let plain = grown(0.0);
        assert_eq!(
            (area_of(&plain, Bridge), area_of(&plain, Bottom)),
            (100.0, 100.0)
        );
        // With 1 mm the bridge takes 1.4 + 1 mm of the bottom.
        let further = grown(1.0);
        assert!((area_of(&further, Bridge) - 124.0).abs() < 0.05);
        assert!((area_of(&further, Bottom) - 76.0).abs() < 0.05);
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
    fn a_top_does_not_reach_across_a_wall_into_fill_it_does_not_touch() {
        // A strip of top fill, then a wall 0.5 mm wide with nothing to
        // fill under it, then solid fill: the strip's walls go round the
        // fill and not the top, as `only_one_wall_top` leaves them.
        let layer = vec![
            Surface::new(Top, block(0.0, 0.0, 3.0, 10.0).remove(0)),
            Surface::new(InternalSolid, block(3.5, 0.0, 13.5, 10.0).remove(0)),
        ];
        let found = anchor(std::slice::from_ref(&layer), &anchoring())
            .unwrap()
            .remove(0);
        assert!((area_of(&found, Top) - 30.0).abs() < 1e-6);
        assert!((area_of(&found, InternalSolid) - 100.0).abs() < 1e-6);
        // Touching the fill, it reaches 1.4 mm into it.
        let layer = vec![
            Surface::new(Top, block(0.0, 0.0, 3.0, 10.0).remove(0)),
            Surface::new(InternalSolid, block(3.0, 0.0, 13.0, 10.0).remove(0)),
        ];
        let found = anchor(std::slice::from_ref(&layer), &anchoring())
            .unwrap()
            .remove(0);
        assert!((area_of(&found, Top) - 44.0).abs() < 0.05);
        assert!((area_of(&found, InternalSolid) - 86.0).abs() < 0.05);
    }

    #[test]
    fn a_bridge_does_not_reach_across_a_wall_into_fill_it_does_not_touch() {
        // A bridge with solid fill on its right, and more solid fill
        // beyond a wall 0.5 mm wide on its left.
        let layer = vec![
            Surface::new(InternalSolid, block(-10.5, 0.0, -0.5, 10.0).remove(0)),
            Surface::new(Bridge, block(0.0, 0.0, 10.0, 10.0).remove(0)),
            Surface::new(InternalSolid, block(10.0, 0.0, 20.0, 10.0).remove(0)),
        ];
        let found = anchor(std::slice::from_ref(&layer), &anchoring())
            .unwrap()
            .remove(0);
        // 1.4 mm onto the fill it touches, none onto the other.
        assert!((area_of(&found, Bridge) - 114.0).abs() < 0.05);
        assert!((area_of(&found, InternalSolid) - 186.0).abs() < 0.05);
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
        let before = classify(
            &slices,
            &slices,
            &slices,
            &[],
            &flat(slices.len()),
            &shells(2, 1),
        )
        .unwrap();
        assert_eq!(found[1], before[1]);
        // A sparse area left too small to be worth a pattern goes solid.
        let mut layers = vec![block(0.0, 0.0, 20.0, 20.0); 4];
        layers.extend(vec![block(8.0, 8.0, 12.0, 12.2); 4]);
        let found = anchored(&layers, &shells(2, 1));
        assert_eq!(area_of(&found[3], Sparse), 0.0);
    }
}
