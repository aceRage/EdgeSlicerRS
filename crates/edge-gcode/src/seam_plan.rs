//! What an outer wall's seam is chosen by, worked out for the whole print
//! before any of it is written: every point of every outermost wall as a
//! place its seam might go, with what the reference measures there.
//!
//! This is the reference's SeamPlacer.cpp, whose `init` does the same
//! before it exports: `gather_seam_candidates` (a `SeamCandidate` for
//! each point of each outer wall) and
//! `calculate_overhangs_and_layer_embedding` (how far each hangs out over
//! the layer beneath). The seam itself is then chosen in `seam.rs`.
//!
//! Not as the reference:
//! - It gathers its candidates object by object and measures each
//!   against that object's own layer beneath; here the plate is one, so
//!   a wall is measured against everything printed beneath it. The two
//!   differ only where one object overhangs another.
//! - An extrusion loop of the reference's repeats its first point at its
//!   end, and a loop cut into stretches over air and stretches that rest
//!   repeats each point it is cut at, so its candidates do; a wall here
//!   is one polygon and each of its points is one candidate.

use std::{collections::HashMap, ops::Range};

use edge_extrusion::ExtrusionLoop;
use rayon::prelude::*;

use crate::{Layer, address, overhang::Below};

/// One place an outer wall's seam might go (SeamPlacer.hpp
/// `SeamCandidate`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Candidate {
    /// Where it is, in mm, and the height its layer was sliced at.
    pub(crate) position: [f32; 3],
    /// Which point of its wall it is.
    pub(crate) corner: u32,
    /// How far the wall's line hangs out over the layer beneath here, in
    /// mm, beyond what a 45 degree slope would; 0 where it rests.
    pub(crate) overhang: f32,
}

/// One outer wall's candidates (SeamPlacer.hpp `Perimeter`): they are
/// `points[start..end]` of its layer, in the order the wall runs
/// counter-clockwise, from the wall's first point.
#[derive(Clone, Debug)]
pub(crate) struct Perimeter {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// Width of the wall's line, in mm.
    pub(crate) flow_width: f32,
}

/// The candidates of one layer.
#[derive(Clone, Debug, Default)]
pub(crate) struct LayerSeams {
    pub(crate) perimeters: Vec<Perimeter>,
    pub(crate) points: Vec<Candidate>,
}

/// Every outer wall's candidates.
#[derive(Default)]
pub(crate) struct Plan {
    /// By physical layer: every filament's walls at one height together.
    layers: Vec<LayerSeams>,
    /// Which of its layer's perimeters each outer wall is, by the layer
    /// and the wall's address.
    walls: HashMap<(usize, usize), u32>,
}

/// The slope a wall may hang out by and still be a good place for a
/// seam, from the vertical (SeamPlacer.hpp `overhang_angle_threshold`).
const OVERHANG_ANGLE: f32 = 45.0 * std::f32::consts::PI / 180.0;

impl Plan {
    /// Gathers the candidates of `layers`, of which `groups` says which
    /// are at one height.
    pub(crate) fn new(layers: &[Layer<'_>], groups: &[Range<usize>]) -> Self {
        let gathered: Vec<(LayerSeams, Vec<usize>)> = groups
            .par_iter()
            .enumerate()
            .map(|(index, group)| gather(&layers[group.clone()], index == 0))
            .collect();
        let mut plan = Self::default();
        for (index, (seams, addresses)) in gathered.into_iter().enumerate() {
            for (perimeter, address) in addresses.into_iter().enumerate() {
                plan.walls.insert((index, address), perimeter as u32);
            }
            plan.layers.push(seams);
        }
        plan
    }

    /// The candidates of an outer wall of physical layer `layer`, and
    /// how wide its line is.
    pub(crate) fn candidates(
        &self,
        layer: usize,
        wall: &ExtrusionLoop,
    ) -> Option<(&[Candidate], f32)> {
        let perimeter = *self.walls.get(&(layer, address(wall)))?;
        let seams = &self.layers[layer];
        let perimeter = &seams.perimeters[perimeter as usize];
        Some((
            &seams.points[perimeter.start..perimeter.end],
            perimeter.flow_width,
        ))
    }

    /// How far each point of an outer wall hangs out over the layer
    /// beneath (see [`Candidate::overhang`]), in the order of the wall's
    /// own points.
    pub(crate) fn overhangs(&self, layer: usize, wall: &ExtrusionLoop) -> Option<Vec<f32>> {
        let (candidates, _) = self.candidates(layer, wall)?;
        let mut found = vec![0.0; wall.polygon.points.len()];
        for candidate in candidates {
            found[candidate.corner as usize] = candidate.overhang;
        }
        Some(found)
    }
}

/// The candidates of one physical layer, and the address of the wall
/// each perimeter is.
fn gather(parts: &[Layer<'_>], first: bool) -> (LayerSeams, Vec<usize>) {
    let mut seams = LayerSeams::default();
    let mut addresses = Vec::new();
    let Some(layer) = parts.first() else {
        return (seams, addresses);
    };
    // Any distance will do for the grid: the distance asked for is exact.
    let below = (!first).then(|| Below::new(layer.below, 1.0)).flatten();
    // The height the layer was sliced at.
    let z = (layer.z - layer.height / 2.0) as f32;
    for part in parts {
        for wall in part.loops {
            let points = &wall.polygon.points;
            // The reference takes a loop for an outer wall by its kind
            // or by its being the outermost (`extract_perimeter_polygons`).
            if wall.depth != 0 || points.len() < 3 {
                continue;
            }
            let flow_width = wall.flow.width as f32;
            let start = seams.points.len();
            let count = points.len();
            // Counter-clockwise from the wall's first point
            // (`process_perimeter_polygon`: `make_counter_clockwise`,
            // which turns a hole's points about).
            for step in 0..count {
                let corner = if wall.is_hole {
                    (count - step) % count
                } else {
                    step
                };
                let position = [
                    edge_geom::unscaled(points[corner].x) as f32,
                    edge_geom::unscaled(points[corner].y) as f32,
                    z,
                ];
                let mut overhang = 0.0;
                if let Some(below) = &below {
                    // `calculate_overhangs_and_layer_embedding`, in the
                    // precision it is worked out in there.
                    let distance = below.distance(position[0].into(), position[1].into());
                    overhang = ((distance + f64::from(0.65 * flow_width)
                        - f64::from(OVERHANG_ANGLE.tan()) * part.height)
                        as f32)
                        .max(0.0);
                }
                seams.points.push(Candidate {
                    position,
                    corner: corner as u32,
                    overhang,
                });
            }
            seams.perimeters.push(Perimeter {
                start,
                end: seams.points.len(),
                flow_width,
            });
            addresses.push(address(wall));
        }
    }
    (seams, addresses)
}

#[cfg(test)]
mod tests {
    use edge_extrusion::{Flow, Role};
    use edge_geom::{ExPolygon, Point2, Polygon};

    use super::*;

    fn ring(corners: &[(f64, f64)]) -> Polygon {
        Polygon {
            points: corners
                .iter()
                .map(|&(x, y)| Point2::from_mm(x, y))
                .collect(),
        }
    }

    fn wall(polygon: Polygon, depth: usize, is_hole: bool) -> ExtrusionLoop {
        ExtrusionLoop {
            polygon,
            role: if depth == 0 {
                Role::ExternalPerimeter
            } else {
                Role::Perimeter
            },
            is_hole,
            depth,
            flow: Flow::new(0.42, 0.2, 0.4).unwrap(),
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: false,
        }
    }

    fn layer<'a>(z: f64, loops: &'a [ExtrusionLoop], below: &'a [ExPolygon]) -> Layer<'a> {
        Layer {
            z,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops,
            islands: &[],
            paths: &[],
            shelter: &[],
            below,
        }
    }

    #[test]
    fn a_point_hangs_out_by_how_far_its_line_is_past_a_45_degree_slope() {
        // A square wall 20 mm a side over a plate that stops 0.1 mm
        // inside it on the left, 1 mm outside it on the right, and well
        // clear above and below.
        let loops = [
            wall(
                ring(&[(0.0, 0.0), (20.0, 0.0), (20.0, 20.0), (0.0, 20.0)]),
                0,
                false,
            ),
            // A wall behind another is no candidate.
            wall(
                ring(&[(0.5, 0.5), (19.5, 0.5), (19.5, 19.5), (0.5, 19.5)]),
                1,
                false,
            ),
        ];
        let plate = [ExPolygon {
            contour: ring(&[(0.1, -5.0), (21.0, -5.0), (21.0, 25.0), (0.1, 25.0)]),
            holes: Vec::new(),
        }];
        let layers = [layer(0.2, &loops, &[]), layer(0.4, &loops, &plate)];
        let plan = Plan::new(&layers, &[0..1, 1..2]);
        assert!(plan.candidates(1, &loops[1]).is_none());
        let (candidates, width) = plan.candidates(1, &loops[0]).unwrap();
        assert_eq!((candidates.len(), width), (4, 0.42));
        // Counter-clockwise from the first point, at the height the
        // layer was sliced at.
        let corners: Vec<u32> = candidates.iter().map(|point| point.corner).collect();
        assert_eq!(corners, [0, 1, 2, 3]);
        assert!(candidates.iter().all(|point| point.position[2] == 0.3));
        // The left corners are 0.1 mm outside the plate: 0.1 + 0.65 of
        // the width less the layer's height hangs out. The right ones
        // are a mm inside.
        for corner in [0, 3] {
            assert!((candidates[corner].overhang - (0.1 + 0.65 * 0.42 - 0.2)).abs() < 1e-5);
        }
        for corner in [1, 2] {
            assert_eq!(candidates[corner].overhang, 0.0);
        }
        assert_eq!(
            plan.overhangs(1, &loops[0]).unwrap(),
            [candidates[0].overhang, 0.0, 0.0, candidates[3].overhang]
        );
        // The same wall is given on the layer beneath, where it has
        // candidates of its own: nothing hangs out on the first layer.
        assert_eq!(plan.overhangs(0, &loops[0]).unwrap(), [0.0; 4]);
        // Nothing hangs out on the first layer, which has nothing
        // beneath it.
        let first = [layer(0.2, &loops, &plate)];
        let plan = Plan::new(&first, std::slice::from_ref(&(0..1)));
        let (candidates, _) = plan.candidates(0, &loops[0]).unwrap();
        assert!(candidates.iter().all(|point| point.overhang == 0.0));
    }

    #[test]
    fn a_holes_candidates_run_the_other_way_round_from_its_first_point() {
        let hole = [wall(
            ring(&[(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)]),
            0,
            true,
        )];
        let layers = [layer(0.2, &hole, &[])];
        let plan = Plan::new(&layers, std::slice::from_ref(&(0..1)));
        let (candidates, _) = plan.candidates(0, &hole[0]).unwrap();
        let corners: Vec<u32> = candidates.iter().map(|point| point.corner).collect();
        assert_eq!(corners, [0, 3, 2, 1]);
    }

    #[test]
    fn the_distance_to_the_outline_beneath_is_told_however_far() {
        let plate = [ExPolygon {
            contour: ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
            holes: vec![ring(&[(4.0, 4.0), (4.0, 6.0), (6.0, 6.0), (6.0, 4.0)])],
        }];
        let below = Below::new(&plate, 0.6).unwrap();
        let near = |found: f64, wanted: f64| (found - wanted).abs() < 1e-9;
        // Over the plate, over its hole, and off it by a little and a lot.
        assert!(near(below.distance(2.0, 5.0), -2.0));
        assert!(near(below.distance(5.0, 5.0), 1.0));
        assert!(near(below.distance(10.25, 5.0), 0.25));
        assert!(near(below.distance(40.0, 5.0), 30.0));
        assert!(near(below.distance(13.0, 14.0), 5.0));
        assert!(near(below.distance(-30.0, -40.0), 50.0));
        // Where the nearer measure tells them apart the two agree.
        assert!(near(below.signed_distance(10.25, 5.0), 0.25));
        assert!(near(below.signed_distance(2.0, 5.0), -0.6));
    }
}
