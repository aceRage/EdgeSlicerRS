//! Overhangs made printable without supports (`make_overhang_printable`):
//! a port of the reference's `PrintObject::apply_conical_overhang`
//! (PrintObjectSlice.cpp).
//!
//! Working from the top layer down, each layer is made to hold the layer
//! above it less a margin: what a layer of that height may reach out past
//! the one beneath at the steepest overhang allowed. Whatever of the layer
//! above is not held up that way is added to the layer below, so the part
//! grows a slope under every overhang, down to where it meets the part
//! again or the bed.
//!
//! The margin is taken off the layer above as a whole, from its outline
//! inward, so the roof of a hole in the underside of a part is not sloped
//! but filled: the hole is closed all the way down. A hole smaller than
//! `make_overhang_printable_hole_size` is kept open instead.

use edge_clip::{ClipError, Join};
use edge_geom::{ExPolygon, Polygon, scaled};

use crate::LayerRegions;

/// Grows `layers` downward so that no layer reaches further past the one
/// beneath it than `angle` allows: degrees from upright, so that 90 leaves
/// the part as it is and 0 lets nothing overhang at all. `heights` holds
/// each layer's thickness in mm. A hole in a layer that the layer above
/// roofs over completely, and that is smaller than `max_hole_area` mm², is
/// left open; with zero every such hole is filled.
///
/// On a layer in several filaments what is added goes to the filament
/// that prints above it, and is taken from any other that was there.
///
/// Layers are taken one after another, each from the one above as it was
/// left, so the result does not depend on how many threads there are.
///
/// Not as the reference:
///
/// - The margin is `tan(angle)` times the height of the layer that
///   overhangs. The reference takes the `layer_height` setting for every
///   layer, so where layers are thinner or thicker than that (variable
///   layer height, `precise_z_height`) its slope is not at the angle
///   asked for.
/// - The reference works object by object and here everything on the
///   plate is one stack of layers. The two differ only where a part
///   floats over nothing: the reference adds nothing to a layer on which
///   that object has nothing, and here a layer counts as empty only when
///   nothing on the plate is on it.
/// - The reference grows every region of a layer by a tenth of a micron
///   before it unites them (`Layer::merged`), and takes a little more than
///   is added from the other regions (`ApplySafetyOffset`), to keep its
///   clipper from leaving slivers where regions meet. A layer here is
///   already one outline, and neither is done.
pub fn make_overhangs_printable(
    layers: &mut [LayerRegions],
    heights: &[f64],
    angle: f64,
    max_hole_area: f64,
) -> Result<(), ClipError> {
    assert_eq!(layers.len(), heights.len(), "a height for every layer");
    if angle >= 90.0 {
        return Ok(());
    }
    let slope = angle.max(0.0).to_radians().tan();
    for index in (0..layers.len().saturating_sub(1)).rev() {
        let (below, above) = layers.split_at_mut(index + 1);
        let (layer, upper) = (&mut below[index], &above[0]);
        if upper.regions.is_empty() || layer.regions.is_empty() {
            continue;
        }
        let mut held = upper.regions.clone();
        // "Avoid closing up of recessed holes in the base of a model":
        // a hole the layer above covers completely is where a hole in
        // the underside ends, and is cut out of the layer above before
        // that is added.
        if max_hole_area > 0.0 {
            for hole in layer.regions.iter().flat_map(|region| &region.holes) {
                if hole.signed_area_mm2().abs() >= max_hole_area {
                    continue;
                }
                let hole = [ExPolygon {
                    contour: Polygon {
                        points: hole.points.iter().rev().copied().collect(),
                    },
                    holes: Vec::new(),
                }];
                if edge_clip::difference(&hole, &held)?.is_empty() {
                    held = edge_clip::difference(&held, &hole)?;
                }
            }
        }
        let reach = scaled(slope * heights[index + 1]);
        let held = edge_clip::offset(&held, -reach, Join::MITER)?;
        // "Remove all islands that have already been fully covered by
        // current layer."
        let uncovered =
            |islands: Vec<ExPolygon>, current: &[ExPolygon]| -> Result<Vec<ExPolygon>, ClipError> {
                let mut kept = Vec::new();
                for island in islands {
                    if !edge_clip::difference(std::slice::from_ref(&island), current)?.is_empty() {
                        kept.push(island);
                    }
                }
                Ok(kept)
            };
        let mut added: Vec<ExPolygon> = Vec::new();
        if upper.by_filament.is_empty() {
            added = uncovered(held, &layer.regions)?;
        } else {
            for (filament, share) in &upper.by_filament {
                let own = uncovered(edge_clip::intersection(share, &held)?, &layer.regions)?;
                if own.is_empty() {
                    continue;
                }
                for (other, theirs) in &mut layer.by_filament {
                    if other != filament {
                        *theirs = edge_clip::difference(theirs, &own)?;
                    }
                }
                let at = match layer
                    .by_filament
                    .binary_search_by_key(filament, |(of, _)| *of)
                {
                    Ok(at) => at,
                    Err(at) => {
                        layer.by_filament.insert(at, (*filament, Vec::new()));
                        at
                    }
                };
                let mine = &mut layer.by_filament[at].1;
                mine.extend(own.iter().cloned());
                *mine = edge_clip::union_ex(mine)?;
                added.extend(own);
            }
        }
        if !added.is_empty() {
            layer.regions.extend(added);
            layer.regions = edge_clip::union_ex(&layer.regions)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use edge_geom::{Point2, unscaled};
    use edge_mesh::{Mesh, primitives::cuboid};

    use super::*;
    use crate::{Body, layer_planes, slice_regions};

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
        Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        }
    }

    fn solid(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: rect(x0, y0, x1, y1),
            holes: Vec::new(),
        }
    }

    fn layer(regions: Vec<ExPolygon>) -> LayerRegions {
        LayerRegions {
            z: 0.0,
            regions,
            by_filament: Vec::new(),
            cut_by_filament: Vec::new(),
            open_chains: 0,
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    /// The least and greatest x of what is on a layer, in mm.
    fn span(regions: &[ExPolygon]) -> (f64, f64) {
        let xs = regions
            .iter()
            .flat_map(|region| &region.contour.points)
            .map(|point| unscaled(point.x));
        xs.fold((f64::MAX, f64::MIN), |(low, high), x| {
            (low.min(x), high.max(x))
        })
    }

    /// A 10 mm stem 10 mm tall under a 30 mm cap 4 mm thick, cut at
    /// 0.2 mm: 70 layers.
    fn mushroom() -> Vec<LayerRegions> {
        let mut soup = cuboid([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]);
        soup.extend(cuboid([0.0, 0.0, 10.0], [30.0, 30.0, 14.0]));
        let mesh = Mesh::from_triangle_soup(soup);
        let body = Body {
            solid: &mesh,
            negative: None,
            filament: 1,
        };
        slice_regions(&[body], &layer_planes(0.0, 14.0, 0.2, 0.2), 0).unwrap()
    }

    #[test]
    fn a_cap_grows_a_slope_down_its_stem() {
        let mut layers = mushroom();
        assert_eq!(layers.len(), 70);
        let before: Vec<f64> = layers.iter().map(|layer| area(&layer.regions)).collect();
        make_overhangs_printable(&mut layers, &[0.2; 70], 45.0, 0.0).unwrap();
        // The cap is as it was, and each layer under it reaches 0.2 mm
        // less far than the one above: 45 degrees.
        for index in 50..70 {
            assert!((area(&layers[index].regions) - before[index]).abs() < 1e-6);
        }
        for (steps, index) in (0..50).rev().enumerate() {
            let side = (30.0 - 0.4 * (steps + 1) as f64).max(10.0);
            let (low, high) = span(&layers[index].regions);
            assert!(
                (high - low - side).abs() < 1e-6,
                "layer {index}: {low} to {high}, wanted {side} across"
            );
            assert!((area(&layers[index].regions) - side * side).abs() < 1e-6);
            assert_eq!(layers[index].regions.len(), 1);
        }
        // 10 mm out at 0.2 mm a layer: the slope meets the stem after
        // 50 layers, exactly at the bed, and the first layer is as cut.
        assert!((area(&layers[0].regions) - 100.0).abs() < 1e-6);
    }

    #[test]
    fn the_angle_sets_how_far_each_layer_may_reach() {
        // 55 degrees from upright, the default: tan(55°) x 0.2 mm a layer.
        let mut layers = mushroom();
        make_overhangs_printable(&mut layers, &[0.2; 70], 55.0, 0.0).unwrap();
        let step = 55f64.to_radians().tan() * 0.2;
        for (steps, index) in (40..50).rev().enumerate() {
            let side = 30.0 - 2.0 * step * (steps + 1) as f64;
            let (low, high) = span(&layers[index].regions);
            assert!((high - low - side).abs() < 1e-5, "layer {index}");
        }
        // The slope has run into the stem 35 layers under the cap.
        assert!((area(&layers[10].regions) - 100.0).abs() < 1e-6);

        // 90 leaves the part alone; 0 carries the cap straight down.
        let mut layers = mushroom();
        let as_cut: Vec<Vec<ExPolygon>> = layers.iter().map(|l| l.regions.clone()).collect();
        make_overhangs_printable(&mut layers, &[0.2; 70], 90.0, 0.0).unwrap();
        assert!(layers.iter().zip(&as_cut).all(|(l, cut)| l.regions == *cut));
        make_overhangs_printable(&mut layers, &[0.2; 70], 0.0, 0.0).unwrap();
        assert!(
            layers
                .iter()
                .all(|layer| (area(&layer.regions) - 900.0).abs() < 1e-6)
        );
    }

    #[test]
    fn the_reach_follows_the_height_of_the_layer_that_overhangs() {
        // The cap's lowest layer twice as thick: the layer under it may
        // be 0.4 mm short of it on every side, the next 0.2 mm more.
        let mut layers = mushroom();
        let mut heights = [0.2; 70];
        heights[50] = 0.4;
        make_overhangs_printable(&mut layers, &heights, 45.0, 0.0).unwrap();
        let (low, high) = span(&layers[49].regions);
        assert!((high - low - 29.2).abs() < 1e-6, "{low} to {high}");
        let (low, high) = span(&layers[48].regions);
        assert!((high - low - 28.8).abs() < 1e-6, "{low} to {high}");
    }

    #[test]
    fn a_part_with_no_overhang_is_left_exactly_as_cut() {
        // A block, and a pyramid of steps each well inside the one below.
        let mut layers: Vec<LayerRegions> = (0..10)
            .map(|index| {
                let inset = index as f64 * 0.1;
                layer(vec![solid(inset, inset, 20.0 - inset, 20.0 - inset)])
            })
            .collect();
        let as_cut: Vec<Vec<ExPolygon>> = layers.iter().map(|l| l.regions.clone()).collect();
        make_overhangs_printable(&mut layers, &[0.2; 10], 55.0, 0.0).unwrap();
        for (layer, cut) in layers.iter().zip(&as_cut) {
            assert_eq!(layer.regions, *cut);
        }
        // An overhang no steeper than the angle is left too: each layer
        // 0.2 mm wider than the one below, at 0.2 mm a layer and 55 degrees.
        let mut layers: Vec<LayerRegions> = (0..10)
            .map(|index| {
                let out = index as f64 * 0.2;
                layer(vec![solid(-out, -out, 20.0 + out, 20.0 + out)])
            })
            .collect();
        let as_cut: Vec<Vec<ExPolygon>> = layers.iter().map(|l| l.regions.clone()).collect();
        make_overhangs_printable(&mut layers, &[0.2; 10], 55.0, 0.0).unwrap();
        for (layer, cut) in layers.iter().zip(&as_cut) {
            assert_eq!(layer.regions, *cut);
        }
    }

    /// A 20 mm block with a 6 mm square hole up into it from underneath,
    /// three layers deep, under two solid layers.
    fn pocket() -> Vec<LayerRegions> {
        let holed = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![Polygon {
                points: rect(7.0, 7.0, 13.0, 13.0)
                    .points
                    .into_iter()
                    .rev()
                    .collect(),
            }],
        };
        let mut layers = vec![layer(vec![holed]); 3];
        layers.extend(vec![layer(vec![solid(0.0, 0.0, 20.0, 20.0)]); 2]);
        layers
    }

    #[test]
    fn a_hole_in_the_underside_is_filled_unless_it_is_small_enough_to_keep() {
        // With no size given every hole the part roofs over is filled,
        // all the way down.
        let mut layers = pocket();
        make_overhangs_printable(&mut layers, &[0.2; 5], 55.0, 0.0).unwrap();
        for layer in &layers {
            assert!((area(&layer.regions) - 400.0).abs() < 1e-6);
            assert!(layer.regions[0].holes.is_empty());
        }
        // 36 mm² is under 40: kept, on every layer it was on.
        let mut layers = pocket();
        make_overhangs_printable(&mut layers, &[0.2; 5], 55.0, 40.0).unwrap();
        for layer in &layers[..3] {
            assert!((area(&layer.regions) - 364.0).abs() < 1e-6);
            assert_eq!(layer.regions[0].holes.len(), 1);
        }
        // And over 30: filled.
        let mut layers = pocket();
        make_overhangs_printable(&mut layers, &[0.2; 5], 55.0, 30.0).unwrap();
        assert!(
            layers
                .iter()
                .all(|layer| (area(&layer.regions) - 400.0).abs() < 1e-6)
        );
    }

    #[test]
    fn a_hole_through_the_part_is_no_overhang() {
        let holed = ExPolygon {
            contour: rect(0.0, 0.0, 20.0, 20.0),
            holes: vec![Polygon {
                points: rect(7.0, 7.0, 13.0, 13.0)
                    .points
                    .into_iter()
                    .rev()
                    .collect(),
            }],
        };
        let mut layers = vec![layer(vec![holed.clone()]); 5];
        make_overhangs_printable(&mut layers, &[0.2; 5], 55.0, 0.0).unwrap();
        assert!(layers.iter().all(|layer| layer.regions == [holed.clone()]));
    }

    #[test]
    fn nothing_is_added_to_a_layer_with_nothing_on_it() {
        // A piece floating two layers over nothing is not carried down
        // to the bed.
        let mut layers = vec![
            layer(Vec::new()),
            layer(Vec::new()),
            layer(vec![solid(0.0, 0.0, 10.0, 10.0)]),
        ];
        make_overhangs_printable(&mut layers, &[0.2; 3], 0.0, 0.0).unwrap();
        assert!(layers[0].regions.is_empty() && layers[1].regions.is_empty());
    }

    #[test]
    fn what_is_added_goes_to_the_filament_above_it() {
        // A stem in filament 1 under a cap that is filament 1 on the
        // left and filament 2 on the right of x = 15.
        let shared = |regions: Vec<ExPolygon>, shares: Vec<(usize, Vec<ExPolygon>)>| LayerRegions {
            z: 0.0,
            regions,
            by_filament: shares,
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        let stem = || {
            shared(
                vec![solid(10.0, 10.0, 20.0, 20.0)],
                vec![(1, vec![solid(10.0, 10.0, 20.0, 20.0)]), (2, Vec::new())],
            )
        };
        let cap = shared(
            vec![solid(0.0, 0.0, 30.0, 30.0)],
            vec![
                (1, vec![solid(0.0, 0.0, 15.0, 30.0)]),
                (2, vec![solid(15.0, 0.0, 30.0, 30.0)]),
            ],
        );
        let mut layers = vec![stem(), stem(), cap];
        make_overhangs_printable(&mut layers, &[0.2; 3], 45.0, 0.0).unwrap();
        // The layer under the cap: 29.6 mm square, split at x = 15, and
        // filament 2 has taken the right half of the stem from filament 1.
        let under = &layers[1];
        assert!((area(&under.regions) - 29.6 * 29.6).abs() < 1e-6);
        let (first, second) = (&under.by_filament[0], &under.by_filament[1]);
        assert_eq!((first.0, second.0), (1, 2));
        assert!((area(&first.1) - 14.8 * 29.6).abs() < 1e-6);
        assert!((area(&second.1) - 14.8 * 29.6).abs() < 1e-6);
        assert!(
            edge_clip::intersection(&first.1, &second.1)
                .unwrap()
                .is_empty()
        );
        assert!((span(&second.1).0 - 15.0).abs() < 1e-6);
        // And the same again a layer further down.
        assert!((area(&layers[0].regions) - 29.2 * 29.2).abs() < 1e-6);
        assert!((area(&layers[0].by_filament[1].1) - 14.6 * 29.2).abs() < 1e-6);
    }

    #[test]
    fn a_filament_new_to_a_layer_takes_its_place_in_order() {
        let cap = LayerRegions {
            z: 0.0,
            regions: vec![solid(0.0, 0.0, 30.0, 30.0)],
            by_filament: vec![(2, vec![solid(0.0, 0.0, 30.0, 30.0)])],
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        let stem = LayerRegions {
            z: 0.0,
            regions: vec![solid(10.0, 10.0, 20.0, 20.0)],
            by_filament: vec![(1, vec![solid(10.0, 10.0, 20.0, 20.0)]), (3, Vec::new())],
            cut_by_filament: Vec::new(),
            open_chains: 0,
        };
        let mut layers = vec![stem, cap];
        make_overhangs_printable(&mut layers, &[0.2; 2], 45.0, 0.0).unwrap();
        let filaments: Vec<usize> = layers[0].by_filament.iter().map(|(of, _)| *of).collect();
        assert_eq!(filaments, [1, 2, 3]);
        assert!(layers[0].by_filament[0].1.is_empty());
        assert!((area(&layers[0].by_filament[1].1) - 29.6 * 29.6).abs() < 1e-6);
    }
}
