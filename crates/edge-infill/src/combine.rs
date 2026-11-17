//! Sparse fill laid once every few layers, as thick as those layers
//! together (`infill_combination`; PrintObject.cpp `combine_infill`).

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon};

use crate::{Surface, SurfaceKind};

/// The sparse fill a layer lays for itself and the layers under it.
#[derive(Clone, Debug, PartialEq)]
pub struct Combined {
    /// How many layers it stands for, this one included.
    pub layers: usize,
    /// Their heights together, in mm: the height its lines are laid at.
    pub thickness: f64,
    /// Where it is laid.
    pub areas: Vec<ExPolygon>,
}

/// Which layers are laid together, as the reference groups them: from
/// the second layer up, layers are added to a group until the next would
/// bring it to `most` mm or more (with libslic3r's `EPSILON` to spare);
/// the group's sparse fill is laid on its top layer. Each entry is the
/// count of layers the layer at that index tops, 0 for none.
pub fn combination_plan(heights: &[f64], most: f64) -> Vec<usize> {
    const EPSILON: f64 = 1e-4;
    let mut plan = vec![0; heights.len()];
    let (mut height, mut count) = (0.0, 0);
    for (index, layer) in heights.iter().enumerate().skip(1) {
        if height + layer >= most + EPSILON {
            plan[index - 1] = count;
            (height, count) = (0.0, 0);
        }
        height += layer;
        count += 1;
    }
    if let Some(last) = plan.last_mut() {
        *last = count;
    }
    plan
}

/// Takes the sparse fill that every layer of a group shares out of the
/// group's layers and gives it to the group's top layer to lay, thick
/// (PrintObject.cpp `combine_infill`). Under the top layer that area is
/// left empty. What is shared is kept a `clearance` off the rest of each
/// layer's sparse fill, so the thin and the thick lines do not cross;
/// a shared piece of no more than `least_area` (units squared) is not
/// worth it and stays as it was.
pub fn combine_infill(
    layers: &mut [Vec<Surface>],
    heights: &[f64],
    plan: &[usize],
    clearance: Coord,
    least_area: f64,
) -> Result<Vec<Option<Combined>>, ClipError> {
    let sparse = |surfaces: &[Surface]| -> Vec<ExPolygon> {
        surfaces
            .iter()
            .filter(|surface| surface.kind == SurfaceKind::Sparse)
            .map(|surface| surface.area.clone())
            .collect()
    };
    let mut combined = vec![None; layers.len()];
    for (top, &count) in plan.iter().enumerate() {
        if count <= 1 {
            continue;
        }
        let group = top + 1 - count..=top;
        let mut shared = edge_clip::union_ex(&sparse(&layers[*group.start()]))?;
        for index in group.clone().skip(1) {
            if shared.is_empty() {
                break;
            }
            shared = edge_clip::intersection(&sparse(&layers[index]), &shared)?;
        }
        if least_area > 0.0 {
            let units_per_mm2 = edge_geom::UNITS_PER_MM * edge_geom::UNITS_PER_MM;
            shared.retain(|area| area.area_mm2() * units_per_mm2 > least_area);
        }
        if shared.is_empty() {
            continue;
        }
        let cleared = edge_clip::offset(&shared, clearance, Join::MITER)?;
        for index in group.clone() {
            let rest = edge_clip::difference(&sparse(&layers[index]), &cleared)?;
            let layer = &mut layers[index];
            layer.retain(|surface| surface.kind != SurfaceKind::Sparse);
            layer.extend(
                rest.into_iter()
                    .map(|area| Surface::new(SurfaceKind::Sparse, area)),
            );
        }
        combined[top] = Some(Combined {
            layers: count,
            thickness: group.map(|index| heights[index]).sum(),
            areas: shared,
        });
    }
    Ok(combined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_geom::{Point2, Polygon};

    fn square(x0: f64, side: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, 0.0), (x0 + side, 0.0), (x0 + side, side), (x0, side)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    #[test]
    fn layers_are_grouped_up_to_the_height_allowed() {
        // 0.2 mm layers under a 0.4 mm nozzle: two at a time; the first
        // layer stands alone, and the last group takes what is left.
        assert_eq!(combination_plan(&[0.2; 8], 0.4), [0, 0, 2, 0, 2, 0, 2, 1]);
        // Three layers of 0.1 come to 0.3, below 0.35; a fourth would not.
        assert_eq!(
            combination_plan(&[0.2, 0.1, 0.1, 0.1, 0.1, 0.1, 0.1], 0.35),
            [0, 0, 0, 3, 0, 0, 3]
        );
        // Nothing to group.
        assert_eq!(combination_plan(&[0.2, 0.2, 0.2], 0.2), [0, 1, 1]);
    }

    #[test]
    fn the_shared_sparse_fill_goes_to_the_top_layer_of_its_group() {
        let layer = |x0: f64| vec![Surface::new(SurfaceKind::Sparse, square(x0, 10.0))];
        let mut layers = vec![layer(0.0), layer(0.0), layer(2.0)];
        let heights = [0.2, 0.2, 0.2];
        let clearance = edge_geom::scaled(0.5);
        let combined = combine_infill(&mut layers, &heights, &[0, 0, 2], clearance, 0.0).unwrap();
        let top = combined[2].as_ref().unwrap();
        assert_eq!((top.layers, (top.thickness * 10.0).round()), (2, 4.0));
        // Shared: from 2 to 10 mm across.
        let area: f64 = top.areas.iter().map(ExPolygon::area_mm2).sum();
        assert!((area - 80.0).abs() < 1e-6, "{area}");
        // Each layer keeps only what lies a clearance off it.
        let left =
            |surfaces: &[Surface]| -> f64 { surfaces.iter().map(|s| s.area.area_mm2()).sum() };
        assert!(
            (left(&layers[1]) - 15.0).abs() < 1e-3,
            "{}",
            left(&layers[1])
        );
        assert!(
            (left(&layers[2]) - 15.0).abs() < 1e-3,
            "{}",
            left(&layers[2])
        );
        assert_eq!(left(&layers[0]), 100.0);
        // Too small a share is left alone.
        let mut again = vec![layer(0.0), layer(0.0), layer(2.0)];
        let none = combine_infill(&mut again, &heights, &[0, 0, 2], clearance, 1e15).unwrap();
        assert!(none.iter().all(Option::is_none));
        assert_eq!(left(&again[1]), 100.0);
    }
}
