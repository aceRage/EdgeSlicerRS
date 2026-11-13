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
//! (MultiMaterialSegmentation.cpp) as it runs with `paint_depth_mode` at
//! `unlimited`, which is its default. The sides are a port of the
//! reference's stages ([`side`], which lists them); the top and bottom
//! faces ([`shells`]) and the merge ([`segment`]) follow it.
//!
//! What the reference does to drop islands of paint too small to print
//! with a classic wall (`drop_unprintable_painted_islands`) is not done.
//! The bounded depths (`paint_depth_mode` at `walls` or `millimeters`,
//! `mmu_segmented_region_max_width`), interlocking and
//! `top_color_penetration_layers` are not here at all; where the bounded
//! depths would go is said at [`segment`].

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

/// What sharing a painted part out needs to know of how it is printed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintSettings {
    /// How many layers a colour painted on a top face is carried down
    /// through, the face's own layer among them, and the same upwards
    /// from a bottom face: the shell layers, with no fewer than the
    /// shell's thickness asks for.
    pub top_layers: usize,
    pub bottom_layers: usize,
    /// The outer wall's width and spacing, in scaled units.
    pub wall_width: Coord,
    pub wall_spacing: Coord,
    /// Painted areas narrower than twice this are dropped: a quarter of
    /// the outer wall's width where gaps are filled, and otherwise half
    /// of what two overlapping walls take up.
    pub small: Coord,
    /// What the lengths runs of colour round an outline are filtered by
    /// are scaled by: 1 for nozzles of 0.4 mm and more, less for finer
    /// ones ([`run_filter_scale`]).
    pub run_scale: f32,
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
/// cross-section at height `planes[i]`, in millimetres, and `facets` all
/// its triangles, painted or not.
pub fn segment(
    outlines: &[&[ExPolygon]],
    planes: &[f64],
    facets: &[Facet],
    settings: &PaintSettings,
) -> Result<Vec<Vec<Share>>, PaintError> {
    assert_eq!(outlines.len(), planes.len(), "one plane per layer");
    let colours = facets.iter().map(|facet| facet.filament).max().unwrap_or(0) as usize + 1;
    if colours == 1 {
        return Ok(vec![Vec::new(); outlines.len()]);
    }
    let cuts = side::cuts(facets, planes);
    let sides: Vec<Vec<Vec<ExPolygon>>> = outlines
        .par_iter()
        .zip(&cuts)
        .enumerate()
        .map(|(layer, (outline, cuts))| {
            side::share(layer, outline, cuts, colours, settings.run_scale)
        })
        .collect::<Result<_, _>>()?;
    // Where paint depth is bounded the reference cuts each colour's
    // share here back to a band along the outline
    // (`cut_segmented_layers`, keeping `paint_depth_clamp_keep_core`),
    // before the faces are worked out and merged in.
    let faces = shells::faces(outlines, planes, facets, colours, settings)?;

    (0..outlines.len())
        .into_par_iter()
        .map(|layer| {
            // Everything a top or bottom face claims on this layer,
            // whatever its colour, unpainted included.
            let claimed: Vec<ExPolygon> = faces[layer].iter().flatten().cloned().collect();
            let mut painted = Vec::new();
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
                if !own.is_empty() {
                    painted.push((colour as u8, own));
                }
            }
            Ok(painted)
        })
        .collect()
}

#[cfg(test)]
mod tests;
