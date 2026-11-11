//! Which filament prints what, where colours are painted onto a part.
//!
//! Paint is on the surface; a print is solid. So each layer is shared out
//! from its outline inwards: every point of a layer takes the colour of
//! the nearest stretch of the layer's outline, and what is painted on a
//! face that looks up or down is carried through the shell behind it,
//! where it wins over what the sides say.
//!
//! This follows the reference's `segmentation_by_painting`
//! (MultiMaterialSegmentation.cpp) as it runs with `paint_depth_mode` at
//! `unlimited`, which is its default: painted triangles are cut by each
//! layer's plane and the cuts matched to the outline's edges, the layer is
//! divided by the Voronoi diagram of those edges, and top and bottom faces
//! are projected onto their layers and taken down or up through the shell
//! layers, further in from the outline with every layer.
//!
//! Not a port line for line. A cut is matched to an edge when it lies
//! within [`NEAR`] of it, where the reference walks a grid and tests
//! angles; a layer's cells are united whole, where the reference traces
//! the borders between colours through the diagram; and what the
//! reference does to drop islands of paint too small to print with a
//! classic wall (`drop_unprintable_painted_islands`) is not done. The
//! bounded depths (`paint_depth_mode` at `walls` or `millimeters`,
//! `mmu_segmented_region_max_width`), interlocking and
//! `top_color_penetration_layers` are not here at all.

mod shells;
mod side;

use edge_clip::ClipError;
use edge_geom::{Coord, ExPolygon};
use edge_medial::MedialError;
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
}

#[derive(Debug, thiserror::Error)]
pub enum PaintError {
    #[error(transparent)]
    Clip(#[from] ClipError),
    #[error("layer {layer}: the painted colours could not be shared out: {source}")]
    Diagram { layer: usize, source: MedialError },
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
        .map(|(layer, (outline, cuts))| side::share(layer, outline, cuts, colours))
        .collect::<Result<_, _>>()?;
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
