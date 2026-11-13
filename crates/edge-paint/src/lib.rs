//! Which filament prints what, where colours are painted onto a part.
//!
//! Paint is on the surface; a print is solid. So each layer is shared out
//! from its outline inwards: the painted triangles colour the stretches
//! of the layer's outline they lie along, the layer is divided between
//! the colours along the borders the Voronoi diagram of those stretches
//! draws, and what is painted on a face that looks up or down is carried
//! through the shell behind it, where it wins over what the sides say.
//!
//! This follows the reference's `segmentation_by_painting`
//! (MultiMaterialSegmentation.cpp). The sides are a port of the
//! reference's stages ([`side`], which lists them); the top and bottom
//! faces ([`shells`]) and the merge ([`segment`]) follow it, and so does
//! how deep a colour reaches where `paint_depth_mode` bounds it
//! ([`depth`]).
//!
//! `top_color_penetration_layers` and `bottom_color_penetration_layers`
//! are not here.

pub mod depth;
mod shells;
mod side;

pub use side::SideError;

use edge_clip::ClipError;
use edge_geom::{Coord, ExPolygon};
use rayon::prelude::*;

/// A triangle in millimetres, wound counter-clockwise seen from outside,
/// and the filament it is painted for, counting from 1; 0 where it is
/// not painted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Facet {
    pub corners: [[f32; 3]; 3],
    pub filament: u8,
}

/// How deep a shell is asked to be: a count of layers and a thickness,
/// whichever takes more layers; none at all where the count is none.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shell {
    pub layers: usize,
    /// In mm.
    pub thickness: f64,
}

/// What sharing a painted part out needs to know of how it is printed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintSettings {
    /// The shells behind a top face and above a bottom face: how far a
    /// colour painted on one is carried, the face's own layer among them.
    pub top: Shell,
    pub bottom: Shell,
    /// The outer wall's width and spacing, in scaled units.
    pub wall_width: Coord,
    pub wall_spacing: Coord,
    /// Painted areas narrower than twice this are dropped: a quarter of
    /// the outer wall's width where gaps are filled, and otherwise half
    /// of what two overlapping walls take up.
    pub small: Coord,
    /// Whether gaps are filled (`gap_infill_speed` above nothing).
    pub gaps_filled: bool,
    /// Whether walls are classic: islands of paint on a face too narrow
    /// for a classic wall's first loop are then dropped.
    pub classic: bool,
    /// What the lengths runs of colour round an outline are filtered by
    /// are scaled by: 1 for nozzles of 0.4 mm and more, less for finer
    /// ones ([`run_filter_scale`]).
    pub run_scale: f32,
    /// How deep paint reaches: unlimited by default.
    pub depth: depth::Depth,
}

/// `mm_segmentation_run_filter_scale`: the lengths runs of colour are
/// filtered by scaled down for the finest nozzle an object is printed
/// with, when that is under 0.4 mm, to no less than a quarter.
pub fn run_filter_scale(nozzle_mm: f64) -> f32 {
    if nozzle_mm.is_nan() || nozzle_mm <= 0.0 || nozzle_mm >= 0.4 - 1e-6 {
        return 1.0;
    }
    (nozzle_mm / 0.4).clamp(0.25, 1.0) as f32
}

#[derive(Debug, thiserror::Error)]
pub enum PaintError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error("layer {layer}: the painted colours could not be shared out: {source}")]
    Diagram {
        layer: usize,
        source: side::SideError,
    },
}

/// What one filament painted onto a part prints of a layer: its number,
/// from 1, and the regions.
pub type Share = (u8, Vec<ExPolygon>);

/// Each layer's painted areas: for every filament painted onto the part
/// that reaches the layer, the part of the layer it prints. What is left
/// of a layer keeps the filament it had. `outlines[i]` is the part's
/// cross-section at height `planes[i]`, in millimetres, `z[i]` the
/// layer's bottom and top, and `facets` all the part's triangles, painted
/// or not.
pub fn segment(
    outlines: &[&[ExPolygon]],
    planes: &[f64],
    z: &[[f64; 2]],
    facets: &[Facet],
    settings: &PaintSettings,
) -> Result<Vec<Vec<Share>>, PaintError> {
    assert_eq!(outlines.len(), planes.len(), "one plane per layer");
    assert_eq!(z.len(), planes.len(), "one bottom and top per layer");
    let colours = facets.iter().map(|facet| facet.filament).max().unwrap_or(0) as usize + 1;
    if colours == 1 {
        return Ok(vec![Vec::new(); outlines.len()]);
    }
    let depth = &settings.depth;
    let cuts = side::cuts(facets, planes);
    // Each colour's share from the sides, cut back to a band along the
    // outline where the depth is bounded (`cut_segmented_layers`), and
    // what that keeps of the part's own filament.
    type Sides = (Vec<Vec<ExPolygon>>, Vec<ExPolygon>);
    let (sides, cores): (Vec<Vec<Vec<ExPolygon>>>, Vec<Vec<ExPolygon>>) = outlines
        .par_iter()
        .zip(&cuts)
        .enumerate()
        .map(|(layer, (outline, cuts))| -> Result<Sides, PaintError> {
            let mut shares = side::share(layer, outline, cuts, colours, settings.run_scale)?;
            let core = depth::cut(layer, outline, &mut shares, depth)?;
            Ok((shares, core))
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .unzip();
    let shells::Faces {
        claims: mut faces,
        legacy,
    } = shells::faces(outlines, planes, z, facets, colours, settings)?;
    // Paint a bounded depth carries from a face deeper than the shell does
    // not take what another painted colour has from the sides
    // (`merge_segmented_layers`, its first part).
    if let Some(legacy) = &legacy {
        faces.par_iter_mut().zip(legacy).zip(&sides).try_for_each(
            |((faces, legacy), sides)| -> Result<(), ClipError> {
                for colour in 1..colours {
                    if faces[colour].is_empty() {
                        continue;
                    }
                    let excess = if legacy[colour].is_empty() {
                        faces[colour].clone()
                    } else {
                        edge_clip::difference(&faces[colour], &legacy[colour])?
                    };
                    if excess.is_empty() {
                        continue;
                    }
                    let others: Vec<ExPolygon> = (1..colours)
                        .filter(|other| *other != colour)
                        .flat_map(|other| sides[other].iter().cloned())
                        .collect();
                    if others.is_empty() {
                        continue;
                    }
                    faces[colour] = edge_clip::difference(
                        &faces[colour],
                        &edge_clip::intersection(&excess, &others)?,
                    )?;
                }
                Ok(())
            },
        )?;
    }

    let absorb = depth.bounded() && depth.min_claim_width > 0.0 && depth.wall_stack > 0.0;
    // How narrow an island beside each colour must be to be given to it:
    // two lines where gap fill is off, one otherwise.
    let widths: Vec<f32> = if settings.gaps_filled {
        Vec::new()
    } else {
        let width = depth
            .min_claim_width
            .max(2.0 * edge_geom::unscaled(settings.small) as f32);
        (0..colours)
            .map(|colour| if colour == 0 { 0.0 } else { width })
            .collect()
    };
    (0..outlines.len())
        .into_par_iter()
        .map(|layer| {
            // Everything a top or bottom face claims on this layer,
            // whatever its colour, unpainted included.
            let claimed: Vec<ExPolygon> = faces[layer].iter().flatten().cloned().collect();
            let mut merged = vec![Vec::new(); colours];
            for colour in 1..colours {
                let mut own = if claimed.is_empty() {
                    sides[layer][colour].clone()
                } else {
                    edge_clip::difference(&sides[layer][colour], &claimed)?
                };
                if !faces[layer][colour].is_empty() {
                    own.extend(faces[layer][colour].iter().cloned());
                    own = edge_clip::intersection(&edge_clip::union_ex(&own)?, outlines[layer])?;
                }
                merged[colour] = own;
            }
            if absorb {
                absorb_between_claims(outlines[layer], &mut merged, &cores[layer], &widths, depth)?;
            }
            Ok(merged
                .into_iter()
                .enumerate()
                .filter(|(_, own)| !own.is_empty())
                .map(|(colour, own)| (colour as u8, own))
                .collect())
        })
        .collect()
}

/// Islands of the part's own filament left between painted claims too
/// narrow to print, each given to the colour beside it that shares most
/// with it (the interior inter-claim absorb at the end of the reference's
/// `merge_segmented_layers`). An island within a wall stack of the
/// outline is left, and so is one in a piece of what the cut back kept on
/// purpose (`core`) where that piece is itself too thin to print.
fn absorb_between_claims(
    outline: &[ExPolygon],
    merged: &mut [Vec<ExPolygon>],
    core: &[ExPolygon],
    widths: &[f32],
    depth: &depth::Depth,
) -> Result<(), ClipError> {
    /// Twice `SCALED_EPSILON`.
    const EPS: Coord = 200;
    let join = edge_clip::Join::MITER;
    if outline.is_empty() {
        return Ok(());
    }
    let painted = edge_clip::union_ex(&merged[1..].concat())?;
    if painted.is_empty() {
        return Ok(());
    }
    let base = edge_clip::difference(outline, &painted)?;
    if base.is_empty() {
        return Ok(());
    }
    let interior = edge_clip::offset(
        outline,
        -depth::delta(depth::scaled_f32(depth.wall_stack)),
        join,
    )?;
    let mut given: Vec<Vec<ExPolygon>> = vec![Vec::new(); merged.len()];
    for island in base {
        let single = std::slice::from_ref(&island);
        let width = depth::absorb_claim_width(single, merged, widths, depth.min_claim_width, EPS)?;
        let t = depth::delta(depth::scaled_f32(width * 0.5));
        if !edge_clip::opening(single, t, join)?.is_empty()
            || !edge_clip::difference(single, &interior)?.is_empty()
        {
            continue;
        }
        let mut kept = false;
        for piece in core {
            let piece = std::slice::from_ref(piece);
            if !edge_clip::intersection(single, piece)?.is_empty()
                && edge_clip::opening(piece, t, join)?.is_empty()
            {
                kept = true;
                break;
            }
        }
        if kept {
            continue;
        }
        let winner = depth::absorb_winner(single, merged, EPS)?;
        if winner > 0 {
            given[winner].push(island);
        }
    }
    for (colour, islands) in given.into_iter().enumerate() {
        if !islands.is_empty() {
            merged[colour].extend(islands);
            merged[colour] = edge_clip::union_ex(&merged[colour])?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod depth_cases;
#[cfg(test)]
mod tests;
