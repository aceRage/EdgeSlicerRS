//! Auto circle compensation: round outlines and round holes are made
//! larger or smaller by an amount worked out from how big they are, to
//! make up for the way a printed circle comes out off size
//! (`enable_circle_compensation`).
//!
//! A port of the reference's `LayerRegion::auto_circle_compensation`
//! (LayerRegion.cpp) and `Polygon::is_approx_circle` (Polygon.cpp), which
//! it has from Bambu Studio. It runs layer by layer once the XY size
//! compensation and the elephant foot compensation are done
//! (PrintObjectSlice.cpp, `PrintObject::slice_volumes`), and before
//! polyholes are looked for.
//!
//! An outline or hole is a circle when it has eight points or more, all
//! within half a millimetre of one distance from its middle, with little
//! spread among those distances. A circle of diameter `d` is moved by
//! `c1 * v + c2 * d + c3`, where `v` is the speed its walls are to be
//! printed at and the three coefficients are the filament's, held between
//! the filament's two limits. For an outline a positive figure is how
//! much it grows; for a hole it is how much the printed hole shrinks, so
//! the hole is opened by it. `circle_compensation_manual_offset` then
//! takes half of itself off either: positive makes a looser fit.
//!
//! A circle under the filament's diameter limit is also marked, for its
//! walls to be printed at the speed the coefficients were measured at.

use edge_clip::{ClipError, Join};
use edge_geom::{Coord, ExPolygon, Point2, Polygon};

use crate::rings::{Replaced, centroid, scale};

/// A filament's part in the compensation, in mm and mm/s.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleCompensation {
    /// The speed the walls of a compensated circle are printed at
    /// (`circle_compensation_speed`).
    pub speed: f64,
    /// `counter_coef_1` to `counter_coef_3`: per mm/s, per mm of
    /// diameter, and outright.
    pub contour_coefs: [f64; 3],
    /// `hole_coef_1` to `hole_coef_3`.
    pub hole_coefs: [f64; 3],
    /// `counter_limit_min` and `counter_limit_max`.
    pub contour_limits: (f64, f64),
    /// `hole_limit_min` and `hole_limit_max`.
    pub hole_limits: (f64, f64),
    /// Circles this large and larger are resized but not marked
    /// (`diameter_limit`).
    pub diameter_limit: f64,
}

impl Default for CircleCompensation {
    /// The reference's defaults (PrintConfig.cpp).
    fn default() -> Self {
        Self {
            speed: 200.0,
            contour_coefs: [0.0, 0.025, -0.11],
            hole_coefs: [0.0, -0.025, 0.28],
            contour_limits: (-0.04, 0.05),
            hole_limits: (0.08, 0.25),
            diameter_limit: 50.0,
        }
    }
}

/// Which rings of a region were resized as circles under the diameter
/// limit (`Surface::counter_circle_compensation`,
/// `Surface::holes_circle_compensation`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Marked {
    pub contour: bool,
    /// Indices into the region's holes.
    pub holes: Vec<usize>,
}

/// A ring found to be a circle (`Polygon::is_approx_circle`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub centre: Point2,
    /// Twice the mean distance of the points from the middle, in units.
    pub diameter: f64,
}

/// The reference's test of a ring, on the points it is judged by.
pub fn approx_circle(points: &[Point2]) -> Option<Circle> {
    let max_deviation = scale(0.5);
    let max_variance = 5.0 * scale(0.01) * scale(0.01);
    if points.len() < 8 {
        return None;
    }
    let centre = centroid(points);
    let reaches: Vec<f64> = points
        .iter()
        .map(|point| {
            let (dx, dy) = ((point.x - centre.x) as f64, (point.y - centre.y) as f64);
            (dx * dx + dy * dy).sqrt()
        })
        .collect();
    let least = reaches.iter().copied().fold(f64::INFINITY, f64::min);
    let most = reaches.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if most - least > max_deviation {
        return None;
    }
    let mean = reaches.iter().sum::<f64>() / reaches.len() as f64;
    let variance = reaches
        .iter()
        .map(|reach| (reach - mean) * (reach - mean))
        .sum::<f64>()
        / reaches.len() as f64;
    if variance > max_variance {
        return None;
    }
    Some(Circle {
        centre,
        diameter: 2.0 * mean,
    })
}

/// How far a circle of `diameter` units is moved, in units, before the
/// manual offset: positive grows an outline, and is how much a hole is
/// opened.
pub fn compensation(coefs: [f64; 3], limits: (f64, f64), speed: f64, diameter: f64) -> f64 {
    let value = scale(coefs[0] * speed) + coefs[1] * diameter + scale(coefs[2]);
    let (least, most) = (scale(limits.0), scale(limits.1));
    value.clamp(least.min(most), most)
}

/// One ring moved out by `delta` units, the way it is wound kept. None
/// when the result is not one ring.
fn moved(ring: &Polygon, delta: f64, is_hole: bool) -> Result<Option<Polygon>, ClipError> {
    let mut solid = ring.clone();
    if is_hole {
        solid.points.reverse();
    }
    let solid = ExPolygon {
        contour: solid,
        holes: Vec::new(),
    };
    // The reference hands the clipper the figure as an `f32`.
    let delta = f64::from(delta as f32).round() as Coord;
    let mut made = edge_clip::offset(std::slice::from_ref(&solid), delta, Join::MITER)?;
    if made.len() != 1 || !made[0].holes.is_empty() {
        return Ok(None);
    }
    Ok(made.pop().map(|region| {
        let mut ring = region.contour;
        if is_hole {
            ring.points.reverse();
        }
        ring
    }))
}

/// Resizes the round outlines and round holes of one filament's share of
/// layer `layer`, in place. Returns what was marked in each region, and
/// which rings were put in place of which.
///
/// `manual_offset` is `circle_compensation_manual_offset` in mm.
pub fn auto_circle_compensation(
    layer: usize,
    regions: &mut [ExPolygon],
    of: &CircleCompensation,
    manual_offset: f64,
) -> Result<(Vec<Marked>, Vec<Replaced>), ClipError> {
    let diameter_limit = scale(of.diameter_limit);
    // The reference carries the manual offset as an `f32`.
    let manual = f64::from(scale(manual_offset) as f32);
    let mut marks = Vec::with_capacity(regions.len());
    let mut replaced = Vec::new();
    for region in regions {
        let mut marked = Marked::default();
        if let Some(circle) = approx_circle(&region.contour.points) {
            let value = compensation(
                of.contour_coefs,
                of.contour_limits,
                of.speed,
                circle.diameter,
            ) - manual / 2.0;
            if let Some(to) = moved(&region.contour, value, false)? {
                replaced.push(Replaced {
                    layer,
                    from: std::mem::replace(&mut region.contour, to.clone()),
                    to,
                });
                marked.contour = circle.diameter < diameter_limit;
            }
        }
        for (index, hole) in region.holes.iter_mut().enumerate() {
            let Some(circle) = approx_circle(&hole.points) else {
                continue;
            };
            // "A positive value is how much the printed hole shrinks, so
            // the hole is opened up by it": the reference offsets the
            // clockwise ring by the figure negated, less half the manual
            // offset, which grows it by the figure and that half.
            let value = compensation(of.hole_coefs, of.hole_limits, of.speed, circle.diameter)
                + manual / 2.0;
            if let Some(to) = moved(hole, value, true)? {
                replaced.push(Replaced {
                    layer,
                    from: std::mem::replace(hole, to.clone()),
                    to,
                });
                if circle.diameter < diameter_limit {
                    marked.holes.push(index);
                }
            }
        }
        marks.push(marked);
    }
    Ok((marks, replaced))
}

#[cfg(test)]
mod tests;
