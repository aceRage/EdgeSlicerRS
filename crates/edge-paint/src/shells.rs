//! Colours from a part's top and bottom faces: what each face is painted,
//! on the layer it closes, and how far that colour is carried into the
//! shell behind it.

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon, UNITS_PER_MM};
use rayon::prelude::*;

use crate::{Facet, PaintError, PaintSettings};

/// A patch of face smaller than this, in mm², is too small to count
/// (the reference's `remove_small`, a tenth of a millimetre squared).
const SMALL_PATCH: f64 = 0.01;
/// A triangle whose shadow on the bed is smaller than this, in mm², is
/// taken to stand upright: neither a top nor a bottom.
const UPRIGHT: f64 = 1e-9;

/// The part of a triangle between two heights, seen from above.
fn slab(corners: &[[f32; 3]; 3], low: f64, high: f64) -> Option<Polygon> {
    let mut points: Vec<[f64; 3]> = corners.iter().map(|c| c.map(f64::from)).collect();
    // Cut to above `low`, then to below `high`.
    for (level, keep_above) in [(low, true), (high, false)] {
        if !level.is_finite() {
            continue;
        }
        let inside = |p: &[f64; 3]| {
            if keep_above {
                p[2] >= level
            } else {
                p[2] <= level
            }
        };
        let mut cut = Vec::with_capacity(points.len() + 2);
        for (at, from) in points.iter().enumerate() {
            let to = &points[(at + 1) % points.len()];
            if inside(from) {
                cut.push(*from);
            }
            if inside(from) != inside(to) {
                let t = (level - from[2]) / (to[2] - from[2]);
                cut.push(std::array::from_fn(|axis| {
                    from[axis] + t * (to[axis] - from[axis])
                }));
            }
        }
        points = cut;
        if points.len() < 3 {
            return None;
        }
    }
    let mut polygon = Polygon {
        points: points
            .iter()
            .map(|p| Point2 {
                x: (p[0] * UNITS_PER_MM).round() as Coord,
                y: (p[1] * UNITS_PER_MM).round() as Coord,
            })
            .collect(),
    };
    polygon.points.dedup();
    let area = polygon.signed_area_mm2();
    if polygon.points.len() < 3 || area.abs() < UPRIGHT {
        return None;
    }
    if area < 0.0 {
        polygon.points.reverse();
    }
    Some(polygon)
}

/// What the faces looking up (or, for `up` false, down) show of each
/// colour on each layer: `[colour][layer]`. A face belongs to the layer
/// whose plane is the last below it (the first above it, looking down),
/// and only what the next layer does not cover is a face at all
/// (TriangleMeshSlicer.cpp `slice_mesh_slabs`, and the trimming in
/// `segmentation_top_and_bottom_layers`).
fn shown(
    outlines: &[&[ExPolygon]],
    planes: &[f64],
    facets: &[Facet],
    colours: usize,
    up: bool,
) -> Result<Vec<Vec<Vec<ExPolygon>>>, ClipError> {
    let count = planes.len();
    let mut patches: Vec<Vec<Vec<Polygon>>> = vec![vec![Vec::new(); count]; colours];
    for facet in facets {
        let [a, b, c] = facet.corners.map(|corner| corner.map(f64::from));
        let facing = (b[0] - a[0]) * (c[1] - a[1]) - (c[0] - a[0]) * (b[1] - a[1]);
        if (facing > 0.0) != up || facing.abs() / 2.0 < UPRIGHT {
            continue;
        }
        let (low, high) = facet
            .corners
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(low, high), corner| {
                (low.min(corner[2].into()), high.max(corner[2].into()))
            });
        for layer in 0..count {
            // Looking up, a layer has what lies from its plane to the
            // next; looking down, from the plane before to its own.
            let (from, to) = if up {
                (
                    planes[layer],
                    planes.get(layer + 1).copied().unwrap_or(f64::INFINITY),
                )
            } else {
                (
                    if layer == 0 {
                        f64::NEG_INFINITY
                    } else {
                        planes[layer - 1]
                    },
                    planes[layer],
                )
            };
            if high < from || low > to {
                continue;
            }
            if let Some(patch) = slab(&facet.corners, from, to) {
                patches[facet.filament as usize][layer].push(patch);
            }
        }
    }
    patches
        .into_par_iter()
        .map(|by_layer| {
            by_layer
                .into_par_iter()
                .enumerate()
                .map(|(layer, patches)| {
                    if patches.is_empty() {
                        return Ok(Vec::new());
                    }
                    let mut face = edge_clip::union(&patches)?;
                    let beyond = if up {
                        outlines.get(layer + 1)
                    } else {
                        layer.checked_sub(1).map(|below| &outlines[below])
                    };
                    if let Some(beyond) = beyond.filter(|beyond| !beyond.is_empty()) {
                        face = edge_clip::difference(&face, beyond)?;
                    }
                    face.retain(|piece| piece.area_mm2() >= SMALL_PATCH);
                    Ok(face)
                })
                .collect()
        })
        .collect()
}

/// What top and bottom faces claim of each layer, by colour, unpainted
/// (0) among them: `[layer][colour]`, with no two colours overlapping.
///
/// A face claims its own layer where it shows, and the layers of shell
/// behind it: on each, what still lies within the outlines of all the
/// layers from the face to it, drawn in by a wall and a space for every
/// layer gone, and besides that the part of the face no wall stands on,
/// drawn in by the one wall. On a layer a face showing there wins over
/// colour carried in from another layer, and of two colours carried in
/// the lower number wins (`segmentation_top_and_bottom_layers`).
pub(crate) fn faces(
    outlines: &[&[ExPolygon]],
    planes: &[f64],
    facets: &[Facet],
    colours: usize,
    settings: &PaintSettings,
) -> Result<Vec<Vec<Vec<ExPolygon>>>, PaintError> {
    let count = planes.len();
    let none = || vec![vec![Vec::new(); colours]; count];
    if settings.top_layers == 0 && settings.bottom_layers == 0 {
        return Ok(none());
    }
    let wall = settings.wall_width + settings.wall_spacing;
    let join = Join::MITER;
    // On its own layer, and carried into the others: [layer][colour].
    let (mut own, mut carried) = (none(), none());
    for up in [true, false] {
        let reach = if up {
            settings.top_layers
        } else {
            settings.bottom_layers
        };
        if reach == 0 {
            continue;
        }
        let shown = shown(outlines, planes, facets, colours, up)?;
        // Each face's claims, as (layer, on its own layer, regions).
        type Claim = (usize, bool, Vec<ExPolygon>);
        let claims: Vec<Vec<Vec<Claim>>> = shown
            .par_iter()
            .map(|by_layer| {
                by_layer
                    .par_iter()
                    .enumerate()
                    .map(|(layer, face)| -> Result<Vec<Claim>, ClipError> {
                        let face = edge_clip::opening(face, settings.small, join)?;
                        if face.is_empty() {
                            return Ok(Vec::new());
                        }
                        // The part of the face with no wall of the next
                        // layer on it.
                        let beyond = if up {
                            outlines.get(layer + 1)
                        } else {
                            layer.checked_sub(1).map(|below| &outlines[below])
                        };
                        let bare = match beyond.filter(|beyond| !beyond.is_empty()) {
                            Some(beyond) => edge_clip::difference(
                                &face,
                                &edge_clip::offset(beyond, wall, join)?,
                            )?,
                            None => face.clone(),
                        };
                        let mut claims = vec![(layer, true, face.clone())];
                        let mut within = outlines[layer].to_vec();
                        let mut inset = 0;
                        // Down from a top, never as far as the first
                        // layer; up from a bottom, to the last.
                        let behind: Vec<usize> = if up {
                            let stop = layer.saturating_sub(reach);
                            (stop + 1..layer).rev().collect()
                        } else {
                            (layer + 1..(layer + reach).min(count)).collect()
                        };
                        for other in behind {
                            inset -= wall;
                            within = edge_clip::intersection(&within, outlines[other])?;
                            let mut reached = edge_clip::intersection(
                                &face,
                                &edge_clip::offset(&within, inset, join)?,
                            )?;
                            if !bare.is_empty() {
                                let reachable = edge_clip::intersection(&bare, &within)?;
                                reached.extend(edge_clip::intersection(
                                    &reachable,
                                    &edge_clip::offset(outlines[other], -wall, join)?,
                                )?);
                                reached = edge_clip::union_ex(&reached)?;
                            }
                            let reached = edge_clip::opening(&reached, settings.small, join)?;
                            if reached.is_empty() {
                                break;
                            }
                            claims.push((other, false, reached));
                        }
                        Ok(claims)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<_, _>>()?;
        for (colour, by_layer) in claims.into_iter().enumerate() {
            for (layer, is_own, regions) in by_layer.into_iter().flatten() {
                let slot = if is_own { &mut own } else { &mut carried };
                slot[layer][colour].extend(regions);
            }
        }
    }

    own.into_par_iter()
        .zip(carried)
        .map(|(own, carried)| -> Result<Vec<Vec<ExPolygon>>, ClipError> {
            let mut claims: Vec<Vec<ExPolygon>> = own
                .iter()
                .map(|regions| edge_clip::union_ex(regions))
                .collect::<Result<_, _>>()?;
            let showing: Vec<ExPolygon> =
                edge_clip::union_ex(&claims.iter().flatten().cloned().collect::<Vec<_>>())?;
            for (claim, carried) in claims.iter_mut().zip(&carried) {
                if carried.is_empty() {
                    continue;
                }
                let behind = edge_clip::difference(&edge_clip::union_ex(carried)?, &showing)?;
                claim.extend(behind);
                *claim = edge_clip::union_ex(claim)?;
            }
            // Painted colours in order, each giving way to those before
            // it; what is unpainted gives way to them all.
            let mut taken: Vec<ExPolygon> = Vec::new();
            for claim in claims.iter_mut().skip(1) {
                if !taken.is_empty() {
                    *claim = edge_clip::difference(claim, &taken)?;
                }
                taken.extend(claim.iter().cloned());
            }
            if !taken.is_empty() {
                claims[0] = edge_clip::difference(&claims[0], &taken)?;
            }
            Ok(claims)
        })
        .collect::<Result<_, _>>()
        .map_err(PaintError::from)
}
