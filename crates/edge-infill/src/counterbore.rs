//! The bridges over a hole that narrows on the way up
//! (`counterbore_hole_bridging`): the areas the walls leave out on the
//! first layer of the narrow part (`edge_perimeter::split_counterbores`)
//! are surfaces of their own, laid as bridges whole.
//!
//! In the reference they go into the layer's fill with everything else
//! (PerimeterGenerator.cpp `process_no_bridge`, "add this directly to
//! the infill list"). What of one has nothing under it is then found to
//! be a bridge (PrintObject.cpp `detect_surfaces_type`), the strip of it
//! that rests on the layer below is sparse fill a wall's spacing wide,
//! and the bridge is grown into that strip, all of it, and turned by the
//! edges it has nothing beside (LayerRegion.cpp
//! `expand_bridges_detect_orientations`). The walls of the layer stand
//! between such an area and the rest of the fill, so nothing else is
//! grown into and nothing else grows into it: here the area is made a
//! bridge as it is and given its direction the same way.

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon};

use crate::surfaces::{BESIDE, Surface, SurfaceKind, bridge_direction};

/// Adds to one layer's surfaces the areas left to be bridged over holes
/// that narrow. `lower` is the whole of the layer below, and `opening`
/// what a strip with nothing under it must be wider than twice of to
/// count (`Shells::opening`). `fixed_angle` is the direction of every
/// bridge where the profile sets one, in radians; `added_angle` what is
/// added to the direction found where it does not (`bridge_angle`, with
/// `relative_bridge_angle`).
///
/// An area takes the direction of the largest part of it that is over
/// nothing: along the edges that part has nothing to rest on beside it,
/// and the short way across where it is held all round.
pub fn bridge_counterbores(
    surfaces: &mut Vec<Surface>,
    bridged: &[ExPolygon],
    lower: &[ExPolygon],
    opening: Coord,
    fixed_angle: Option<f64>,
    added_angle: f64,
) -> Result<(), ClipError> {
    let join = Join::MITER;
    for area in bridged {
        let own = std::slice::from_ref(area);
        let over_air = edge_clip::opening(&edge_clip::difference(own, lower)?, opening, join)?;
        let Some(widest) = over_air
            .iter()
            .max_by(|a, b| a.area_mm2().total_cmp(&b.area_mm2()))
        else {
            // All of it rests on the layer below: nothing to span.
            surfaces.push(Surface::new(SurfaceKind::InternalSolid, area.clone()));
            continue;
        };
        let angle = match fixed_angle {
            Some(angle) => angle,
            None => {
                let held = edge_clip::intersection(own, lower)?;
                let beside = edge_clip::offset(&held, BESIDE, join)?;
                bridge_direction(widest, &beside) + added_angle
            }
        };
        surfaces.push(Surface {
            kind: SurfaceKind::Bridge,
            area: area.clone(),
            angle: Some(angle.rem_euclid(std::f64::consts::PI)),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::f64::consts::{FRAC_PI_2, PI};

    use edge_geom::{Point2, Polygon};

    use super::*;

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

    /// A slab with a pit 10 mm square in it.
    fn slab() -> Vec<ExPolygon> {
        let mut pit = rect(0.0, 0.0, 10.0, 10.0);
        pit.points.reverse();
        vec![ExPolygon {
            contour: rect(-10.0, -10.0, 20.0, 20.0),
            holes: vec![pit],
        }]
    }

    #[test]
    fn a_strip_across_the_pit_is_bridged_along_its_open_edge() {
        // A strip along one side of the pit, resting on the slab at both
        // ends and down one side by 0.4 mm; its fourth edge is in the air.
        let strip = solid(-0.4, -0.4, 3.0, 10.4);
        let mut surfaces = vec![Surface::new(
            SurfaceKind::Sparse,
            solid(12.0, 0.0, 18.0, 10.0),
        )];
        bridge_counterbores(
            &mut surfaces,
            std::slice::from_ref(&strip),
            &slab(),
            42_000,
            None,
            0.0,
        )
        .unwrap();
        assert_eq!(surfaces.len(), 2);
        assert_eq!(surfaces[0].kind, SurfaceKind::Sparse);
        let bridge = &surfaces[1];
        assert_eq!(bridge.kind, SurfaceKind::Bridge);
        // All of it, the part resting on the slab included.
        assert_eq!(bridge.area, strip);
        let angle = bridge.angle.unwrap();
        assert!((angle - FRAC_PI_2).abs() < 1e-6, "{angle}");
        // A turn added to the direction found, and kept under a half turn.
        let mut turned = Vec::new();
        bridge_counterbores(
            &mut turned,
            std::slice::from_ref(&strip),
            &slab(),
            42_000,
            None,
            0.75 * PI,
        )
        .unwrap();
        let angle = turned[0].angle.unwrap();
        assert!((angle - 0.25 * PI).abs() < 1e-6, "{angle}");
        // One direction for every bridge.
        let mut fixed = Vec::new();
        bridge_counterbores(&mut fixed, &[strip], &slab(), 42_000, Some(0.3), 1.0).unwrap();
        assert_eq!(fixed[0].angle, Some(0.3));
    }

    #[test]
    fn a_cover_held_all_round_is_crossed_the_short_way() {
        // The pit made 10 by 6 mm, covered whole with 0.4 mm to rest on.
        let mut pit = rect(0.0, 0.0, 10.0, 6.0);
        pit.points.reverse();
        let lower = vec![ExPolygon {
            contour: rect(-10.0, -10.0, 20.0, 20.0),
            holes: vec![pit],
        }];
        let cover = solid(-0.4, -0.4, 10.4, 6.4);
        let mut surfaces = Vec::new();
        bridge_counterbores(&mut surfaces, &[cover], &lower, 42_000, None, 0.0).unwrap();
        assert_eq!(surfaces[0].kind, SurfaceKind::Bridge);
        let angle = surfaces[0].angle.unwrap();
        assert!((angle - FRAC_PI_2).abs() < 1e-6, "{angle}");
    }

    #[test]
    fn an_area_with_everything_under_it_is_not_a_bridge() {
        let mut surfaces = Vec::new();
        let area = solid(12.0, 0.0, 18.0, 10.0);
        bridge_counterbores(&mut surfaces, &[area], &slab(), 42_000, None, 0.0).unwrap();
        assert_eq!(surfaces[0].kind, SurfaceKind::InternalSolid);
        assert_eq!(surfaces[0].angle, None);
    }
}
