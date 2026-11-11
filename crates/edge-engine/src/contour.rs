//! Z contouring (`zaa_enabled`) as the pipeline does it: where each layer
//! is sliced, and every layer's walls and lines contoured once the fill
//! and ironing are made (Print.cpp: "after infill and ironing, before
//! path simplification"). `edge_contour` has the rest.

use edge_3mf::Body;
use edge_contour::{At, LayerProfiles, Surface, ZaaSettings};
use edge_extrusion::ExtrusionPath;
use edge_perimeter::Perimeters;
use rayon::prelude::*;

use crate::{Failure, layer_heights::LayerHeights};

/// Moves every layer's slicing plane but the first's to `zaa_min_z`
/// above its bottom. `floor` is the height the print stands on.
pub(crate) fn plane(
    zaa: &ZaaSettings,
    planes: &mut [f64],
    heights: &LayerHeights,
    floor: f64,
) -> Result<(), Failure> {
    for (index, plane) in planes.iter_mut().enumerate().skip(1) {
        let height = heights.height_of(index);
        *plane = zaa.slice_at(index, floor + heights.top_of(index) - height, height)?;
    }
    Ok(())
}

/// Contours the walls and lines of every layer but the first ("to avoid
/// a build plate collision", PrintObject.cpp `contour_z`), each layer on
/// a thread of its own, and gives what each is laid by: by filament, as
/// `walls_by` and `fills_by` are, then by layer.
pub(crate) fn apply(
    zaa: &ZaaSettings,
    bodies: &[Body],
    planes: &[f64],
    heights: &LayerHeights,
    floor: f64,
    walls_by: &mut [Vec<Perimeters>],
    fills_by: &mut [Vec<Vec<ExtrusionPath>>],
) -> Vec<Vec<LayerProfiles>> {
    let surface = Surface::new(bodies.iter().map(|body| &body.solid));
    walls_by
        .iter_mut()
        .zip(fills_by.iter_mut())
        .map(|(walls, fills)| {
            walls
                .par_iter_mut()
                .zip(fills.par_iter_mut())
                .enumerate()
                .map(|(index, (walls, fills))| {
                    if index == 0 {
                        return LayerProfiles::default();
                    }
                    let at = At {
                        print_z: floor + heights.top_of(index),
                        slice_z: planes[index],
                        height: heights.height_of(index),
                    };
                    edge_contour::contour_layer(&mut walls.loops, fills, &at, zaa, &surface)
                })
                .collect()
        })
        .collect()
}
