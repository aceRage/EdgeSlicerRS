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

/// `EPSILON` of the reference, in mm.
const EPSILON: f64 = 1e-4;

/// How many layers a shell behind the face on layer `face` takes, the
/// face's own among them: `layers`, or more where they are thinner than
/// `thickness` together, walked over the layers as they lie; none where
/// `layers` is none (`effective_shell_layers_by_thickness`). `z` holds each
/// layer's bottom and top.
pub(crate) fn shell_layers(
    z: &[[f64; 2]],
    face: usize,
    up: bool,
    layers: usize,
    thickness: f64,
) -> usize {
    if layers == 0 || thickness <= 0.0 {
        return layers;
    }
    let mut walked = 0;
    if up {
        let base = z[face][1];
        let mut ran_off = true;
        for other in (0..face).rev() {
            walked += 1;
            if base - z[other][1] >= thickness - EPSILON {
                ran_off = false;
                break;
            }
        }
        if ran_off {
            walked += 1;
        }
    } else {
        let base = z[face][0];
        let mut ran_off = true;
        for other in &z[face + 1..] {
            walked += 1;
            if other[0] - base >= thickness - EPSILON {
                ran_off = false;
                break;
            }
        }
        if ran_off {
            walked += 1;
        }
    }
    layers.max(walked)
}

/// What top and bottom faces claim of each layer, by colour.
pub(crate) struct Faces {
    /// `[layer][colour]`, unpainted (0) among them, no two colours
    /// overlapping.
    pub claims: Vec<Vec<Vec<ExPolygon>>>,
    /// Where the depth is bounded, each painted colour's claims as far as
    /// the shell alone takes them, not trimmed by the other colours'
    /// (`legacy_top_and_bottom_layers`): `[layer][colour]`.
    pub legacy: Option<Vec<Vec<Vec<ExPolygon>>>>,
}

/// One face's claims, as (layer, kind, regions).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// On the face's own layer.
    Own,
    /// On a layer behind it, within the shell.
    Shell,
    /// On a layer behind it, past the shell: only where a bounded depth
    /// carries paint deeper than the shell.
    Deeper,
}

/// What top and bottom faces claim of each layer, by colour, unpainted
/// (0) among them (`segmentation_top_and_bottom_layers`).
///
/// A face claims its own layer where it shows, and the layers of shell
/// behind it: on each, what still lies within the outlines of all the
/// layers from the face to it, drawn in by a wall and a space for every
/// layer gone, and besides that the part of the face no wall stands on,
/// drawn in by the one wall. On a layer a face showing there wins over
/// colour carried in from another layer, and of two colours carried in
/// the lower number wins.
///
/// Where the depth is bounded and at least a wall stack, a painted face
/// is carried as deep as the depth is thick, whole as far as a wall stack
/// from each layer's outline (a shell of even thickness measured square
/// to the surface, whatever its slope), but a flat face no deeper than
/// its shell; and slivers are filtered from what all faces carry to a
/// layer together rather than from each step.
pub(crate) fn faces(
    outlines: &[&[ExPolygon]],
    planes: &[f64],
    z: &[[f64; 2]],
    facets: &[Facet],
    colours: usize,
    settings: &PaintSettings,
) -> Result<Faces, PaintError> {
    let count = planes.len();
    let none = || vec![vec![Vec::new(); colours]; count];
    let depth = &settings.depth;
    let bounded = depth.bounded();
    if settings.top.layers == 0 && settings.bottom.layers == 0 {
        return Ok(Faces {
            claims: none(),
            legacy: bounded.then(none),
        });
    }
    let wall = settings.wall_width + settings.wall_spacing;
    // Painted colours are carried as deep as the depth where it is at
    // least a wall stack (`LayerColorStat::normal_shell`), give or take
    // `SCALED_EPSILON`.
    let normal = bounded
        && wall > 0
        && f64::from(crate::depth::scaled_f32(depth.normal)) + 100.0 >= wall as f64;
    let join = Join::MITER;
    // On its own layer, and carried into the others from above and from
    // below, as far as the shell takes it (and past it): [layer][colour].
    let (mut own, mut legacy) = (none(), none());
    let mut carried = [none(), none()];
    for up in [true, false] {
        let shell = if up { settings.top } else { settings.bottom };
        if shell.layers == 0 {
            continue;
        }
        let shown = shown(outlines, planes, facets, colours, up)?;
        type Claim = (usize, Kind, Vec<ExPolygon>);
        let claims: Vec<Vec<Vec<Claim>>> = shown
            .par_iter()
            .enumerate()
            .map(|(colour, by_layer)| {
                by_layer
                    .par_iter()
                    .enumerate()
                    .map(|(layer, face)| -> Result<Vec<Claim>, ClipError> {
                        let face = edge_clip::opening(face, settings.small, join)?;
                        let face = if settings.classic && colour > 0 {
                            drop_unprintable(&face, settings, outlines[layer], wall)?
                        } else {
                            face
                        };
                        if face.is_empty() {
                            return Ok(Vec::new());
                        }
                        let shell_layers =
                            shell_layers(z, layer, up, shell.layers, shell.thickness);
                        let deep = normal && colour > 0;
                        let descent = if deep {
                            shell_layers.max(shell_layers_of(z, layer, up, depth.normal))
                        } else {
                            shell_layers
                        };
                        // The layer next to the face, beyond it.
                        let beyond = if up {
                            outlines.get(layer + 1)
                        } else {
                            layer.checked_sub(1).map(|below| &outlines[below])
                        };
                        let beyond = beyond.filter(|beyond| !beyond.is_empty());
                        // The part of the face no wall of the next layer
                        // stands on (`exposed_surface_part`), all of it
                        // where the depth carries the face whole.
                        let bare = match beyond {
                            Some(beyond) if !deep => edge_clip::difference(
                                &face,
                                &edge_clip::offset(beyond, wall, join)?,
                            )?,
                            _ => face.clone(),
                        };
                        // Where the depth goes deeper than the shell, the
                        // flat parts of the face stop at the shell.
                        let flat = if deep && descent > shell_layers {
                            flat_cap(&face, beyond.copied(), wall)?
                        } else {
                            Vec::new()
                        };
                        let mut claims = vec![(layer, Kind::Own, face.clone())];
                        let mut within = outlines[layer].to_vec();
                        let mut inset = 0;
                        // Down from a top, never as far as the first
                        // layer; up from a bottom, to the last.
                        let behind: Vec<usize> = if up {
                            let stop = layer.saturating_sub(descent);
                            (stop + 1..layer).rev().collect()
                        } else {
                            (layer + 1..(layer + descent).min(count)).collect()
                        };
                        let mut deposited = false;
                        for other in behind {
                            inset -= wall;
                            within = edge_clip::intersection(&within, outlines[other])?;
                            let mut reached = edge_clip::intersection(
                                &face,
                                &edge_clip::offset(&within, inset, join)?,
                            )?;
                            if !bare.is_empty() {
                                let reachable = edge_clip::intersection(&bare, &within)?;
                                if deep && reachable.is_empty() {
                                    break;
                                }
                                reached.extend(edge_clip::intersection(
                                    &reachable,
                                    &edge_clip::offset(outlines[other], -wall, join)?,
                                )?);
                                reached = edge_clip::union_ex(&reached)?;
                            }
                            let steps = other.abs_diff(layer);
                            if !flat.is_empty() && steps >= shell_layers {
                                reached = edge_clip::difference(&reached, &flat)?;
                            }
                            if !deep {
                                reached = edge_clip::opening(&reached, settings.small, join)?;
                            }
                            if reached.is_empty() {
                                if deep && !deposited {
                                    continue;
                                }
                                break;
                            }
                            deposited = true;
                            let kind = if steps < shell_layers {
                                Kind::Shell
                            } else {
                                Kind::Deeper
                            };
                            claims.push((other, kind, reached));
                        }
                        Ok(claims)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<_, _>>()?;
        let side = usize::from(!up);
        for (colour, by_layer) in claims.into_iter().enumerate() {
            for (layer, kind, regions) in by_layer.into_iter().flatten() {
                match kind {
                    Kind::Own => own[layer][colour].extend(regions),
                    Kind::Shell => {
                        if bounded {
                            legacy[layer][colour].extend(regions.iter().cloned());
                        }
                        carried[side][layer][colour].extend(regions);
                    }
                    Kind::Deeper => carried[side][layer][colour].extend(regions),
                }
            }
        }
    }

    let [from_top, from_bottom] = carried;
    let small = settings.small;
    let claims = own
        .par_iter()
        .zip(from_top)
        .zip(from_bottom)
        .map(
            |((own, from_top), from_bottom)| -> Result<Vec<Vec<ExPolygon>>, ClipError> {
                let mut claims: Vec<Vec<ExPolygon>> = own
                    .iter()
                    .map(|regions| edge_clip::union_ex(regions))
                    .collect::<Result<_, _>>()?;
                let showing: Vec<ExPolygon> =
                    edge_clip::union_ex(&claims.iter().flatten().cloned().collect::<Vec<_>>())?;
                for (colour, claim) in claims.iter_mut().enumerate() {
                    let (top, bottom) = (&from_top[colour], &from_bottom[colour]);
                    if top.is_empty() && bottom.is_empty() {
                        continue;
                    }
                    let behind = if normal && colour > 0 {
                        // Filtered as a band, top and bottom apart.
                        let mut behind =
                            edge_clip::opening(&edge_clip::union_ex(top)?, small, join)?;
                        behind.extend(edge_clip::opening(
                            &edge_clip::union_ex(bottom)?,
                            small,
                            join,
                        )?);
                        behind
                    } else {
                        top.iter().chain(bottom).cloned().collect()
                    };
                    let behind = edge_clip::difference(&edge_clip::union_ex(&behind)?, &showing)?;
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
            },
        )
        .collect::<Result<_, _>>()?;
    // How far each painted colour reaches with the shell alone: its own
    // faces and what they carry within the shell, filtered as the claims
    // are where the depth carries paint deeper.
    let legacy = if bounded {
        Some(
            own.into_par_iter()
                .zip(legacy)
                .map(|(own, legacy)| -> Result<Vec<Vec<ExPolygon>>, ClipError> {
                    own.into_iter()
                        .zip(legacy)
                        .enumerate()
                        .map(|(colour, (mut reach, legacy))| {
                            reach.extend(legacy);
                            if reach.is_empty() {
                                return Ok(reach);
                            }
                            let reach = edge_clip::union_ex(&reach)?;
                            if normal && colour > 0 {
                                edge_clip::opening(&reach, small, join)
                            } else {
                                Ok(reach)
                            }
                        })
                        .collect()
                })
                .collect::<Result<_, _>>()?,
        )
    } else {
        None
    };
    Ok(Faces { claims, legacy })
}

/// How many layers a face reaches through to cover `mm`, the face's own
/// among them: [`shell_layers`] with one layer asked for.
fn shell_layers_of(z: &[[f64; 2]], face: usize, up: bool, mm: f32) -> usize {
    shell_layers(z, face, up, 1, f64::from(mm))
}

/// The flat parts of a face: each piece of it more than a wall stack from
/// the next layer's outline (or all of it, with none) that is still there
/// opened by a wall stack (by three quarters of one, with no next layer),
/// grown back by two and kept within the face (`flat_cap_component_ex`).
fn flat_cap(
    face: &[ExPolygon],
    beyond: Option<&[ExPolygon]>,
    wall: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    let join = Join::MITER;
    let (exposed, opening) = match beyond {
        Some(beyond) if wall > 0 => (
            edge_clip::difference(face, &edge_clip::offset(beyond, wall, join)?)?,
            wall,
        ),
        _ => (face.to_vec(), (0.75 * wall as f64).round() as Coord),
    };
    if exposed.is_empty() {
        return Ok(Vec::new());
    }
    edge_clip::intersection(
        face,
        &edge_clip::offset(
            &edge_clip::opening(&exposed, opening, join)?,
            2 * wall,
            join,
        )?,
    )
}

/// A painted face's islands with no room for a classic wall's first loop
/// that lie a wall stack or more inside the layer, dropped whole
/// (`drop_unprintable_painted_islands`): classic walls print nothing in
/// them, so they would be taken from the part's own filament and left
/// empty.
fn drop_unprintable(
    face: &[ExPolygon],
    settings: &PaintSettings,
    layer: &[ExPolygon],
    edge_band: Coord,
) -> Result<Vec<ExPolygon>, ClipError> {
    /// `INSET_OVERLAP_TOLERANCE`.
    const OVERLAP: f64 = 0.4;
    let core =
        0.5 * (settings.wall_width as f64 + (1.0 - OVERLAP) * settings.wall_spacing as f64) - 100.0;
    if face.is_empty() || core <= 0.0 || edge_band <= 0 {
        return Ok(face.to_vec());
    }
    let join = Join::MITER;
    let mut points: Vec<Point2> = Vec::new();
    let mut add = |pieces: Vec<ExPolygon>| -> Result<(), ClipError> {
        for piece in pieces {
            let inner = edge_clip::offset(std::slice::from_ref(&piece), -100, join)?;
            let used = inner.first().unwrap_or(&piece);
            if let Some(&point) = used.contour.points.first() {
                points.push(point);
            }
        }
        Ok(())
    };
    add(edge_clip::offset(face, -(core.round() as Coord), join)?)?;
    add(edge_clip::difference(
        face,
        &edge_clip::offset(layer, -edge_band, join)?,
    )?)?;
    if points.is_empty() {
        return Ok(Vec::new());
    }
    Ok(face
        .iter()
        .filter(|piece| {
            points
                .iter()
                .any(|&point| crate::depth::contains(piece, point))
        })
        .cloned()
        .collect())
}
