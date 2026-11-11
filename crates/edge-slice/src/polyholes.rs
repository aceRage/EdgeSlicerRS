//! Polyholes: a round hole that runs straight up through the part is
//! walled as a polygon of few sides drawn round the circle, so that the
//! hole comes out the size it was drawn (`hole_to_polyhole`; see
//! <http://hydraraptor.blogspot.com/2011/02/polyholes.html>).
//!
//! A port of the reference's `PrintObject::_transform_hole_to_polyholes`
//! and `create_polyholes` (PrintObject.cpp, themselves from SuperSlicer).
//! It runs once every layer has its final outline: after the XY size
//! compensation, the elephant foot compensation and the circle
//! compensation (PrintObjectSlice.cpp, `PrintObject::slice`, which calls
//! it after `slice_volumes`).
//!
//! In outline: every hole of every layer is measured from its middle. One
//! with more than eight points, convex, whose points and the middles of
//! whose sides all lie within the margin of one distance is round. Round
//! holes of consecutive layers with the same middle and radius, within the
//! margin, make one run; a run of two layers or more, or a hole of the
//! first layer by itself (elephant foot compensation makes that one
//! larger than those above), is replaced by a regular polygon whose flats
//! touch the circle. Its number of sides comes from the radius and the
//! nozzle, and with `hole_to_polyhole_twisted` it is turned by a fifth of
//! a side from layer to layer.
//!
//! Numbers are kept the way the reference keeps them: the radius in `f32`
//! and cut to whole units, the margin cut to whole units, the corners of
//! the polygon worked out in `f32`. Two things differ on purpose and are
//! said where they are: the test for a convex hole, which in the
//! reference never fails, and runs with one and the same middle and
//! radius, of which the reference converts only the first.

use std::f32::consts::PI as PI_F32;

use edge_geom::{Coord, ExPolygon, Point2, Polygon};
use rayon::prelude::*;

use crate::rings::{Replaced, SCALING_FACTOR, centroid, distance, scale};

/// `SCALED_EPSILON`: the least the margin can be.
const SCALED_EPSILON: f64 = scale(1e-4);
/// A hole must run through this many layers to be converted, unless it
/// is on the first (`min_nb_layers`).
const MIN_LAYERS: usize = 2;
/// With the twist, a polyhole comes back to where it was after this many
/// layers (`nb_polyhole`).
const TWIST_STEPS: usize = 5;

/// How far a hole's points may stray from one radius and still be round
/// (`hole_to_polyhole_threshold`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Margin {
    Mm(f64),
    /// Of the hole's mean radius.
    Percent(f64),
}

/// What `hole_to_polyhole` is to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Polyholes {
    pub margin: Margin,
    /// Each layer's polygon is turned from the one below
    /// (`hole_to_polyhole_twisted`).
    pub twisted: bool,
    /// The most sides a polyhole may have (`hole_to_polyhole_max_edges`),
    /// 3 or more.
    pub max_edges: i32,
}

/// One filament's share of a layer, and the nozzle that filament is
/// printed through, in mm: the reference looks at a layer region by
/// region, and sizes a polyhole by the nozzle of the region's wall
/// filament.
pub struct Share<'a> {
    pub filament: usize,
    pub nozzle_diameter: f64,
    pub regions: &'a mut [ExPolygon],
}

/// A hole found round.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundHole {
    /// The middle of the hole's area.
    pub centre: Point2,
    /// The distance of the furthest point from the middle, as the
    /// reference holds it (in `f32`, and named a diameter there).
    pub radius: f32,
    /// The margin, in whole units: how far the points may stray, and how
    /// near a hole on the next layer must be to be the same hole.
    pub margin: Coord,
}

/// Judges one hole, given as the layer holds it: clockwise.
pub fn round_hole(hole: &Polygon, margin: Margin) -> Option<RoundHole> {
    let points = &hole.points;
    if points.len() <= 8 {
        return None;
    }
    // "test if convex (as it's clockwise bc it's a hole, we have to do the
    // opposite)": a hole is convex when its ring, gone round clockwise,
    // never turns left.
    //
    // Not as the reference, where the test never fails. It asks for
    // `hole.convex_points(PI)`, the points that turn left by more than
    // the angle given, and no turn is by more than a half circle
    // (Polygon.cpp `filter_convex_concave_points_by_angle_threshold`: a
    // dot product of two unit vectors under cos(PI) = -1), so the list is
    // always empty and a hole with a notch in it passes. The test is
    // made here as it is plainly meant: a hole that is not convex is
    // left alone. A point on a flat counts for nothing, though: one that
    // rounding has left a unit or two to the wrong side of the line
    // through its neighbours (elephant foot compensation puts points
    // along the flats of a large hole) is no notch. A left turn counts
    // once the point stands `SCALED_EPSILON` off that line, the least
    // distance the reference tells apart. The roundness test after it
    // rejects nearly every hole that is not convex anyway; what differs
    // is a hole with dents shallower than the margin.
    let dented = |index: usize| {
        let before = points[(index + points.len() - 1) % points.len()];
        let at = points[index];
        let after = points[(index + 1) % points.len()];
        let v1 = ((at.x - before.x) as f64, (at.y - before.y) as f64);
        let v2 = ((after.x - at.x) as f64, (after.y - at.y) as f64);
        let cross = v1.0 * v2.1 - v1.1 * v2.0;
        // The cross product is the chord's length times how far the
        // point stands off it.
        cross > 0.0 && cross > SCALED_EPSILON * distance(before, after)
    };
    if (0..points.len()).any(dented) {
        return None;
    }
    let centre = centroid(points);
    let (mut least, mut most, mut sum) = (f64::from(f32::MAX), 0.0_f64, 0.0);
    for &point in points {
        let reach = distance(point, centre);
        least = least.min(reach);
        most = most.max(reach);
        sum += reach;
    }
    // "also use center of lines to check it's not a rectangle"
    let (mut least_side, mut most_side) = (f64::from(f32::MAX), 0.0_f64);
    for (index, &from) in points.iter().enumerate() {
        let to = points[(index + 1) % points.len()];
        let middle = Point2 {
            x: (from.x + to.x) / 2,
            y: (from.y + to.y) / 2,
        };
        let reach = distance(centre, middle);
        least_side = least_side.min(reach);
        most_side = most_side.max(reach);
    }
    let mean = sum / points.len() as f64 * SCALING_FACTOR;
    let allowed = match margin {
        Margin::Mm(mm) => mm,
        Margin::Percent(percent) => mean * percent / 100.0,
    };
    let margin = SCALED_EPSILON.max(scale(allowed)) as Coord;
    let twice = (margin * 2) as f64;
    (most - least < twice && most_side - least_side < twice).then_some(RoundHole {
        centre,
        radius: most as f32,
        margin,
    })
}

/// How many sides a polyhole of this radius has for this nozzle, both in
/// units (`create_polyholes`: "n = max(round(2 * d), 3); // for 0.4mm
/// nozzle").
pub fn polyhole_edges(radius: Coord, nozzle_diameter: Coord, max_edges: i32) -> usize {
    let unscaled = |units: Coord| units as f64 * SCALING_FACTOR;
    let wanted = (4.0 * unscaled(radius) * 0.4 / unscaled(nozzle_diameter)).round() as i32;
    max_edges.min(wanted.max(3)).max(3) as usize
}

/// The polygons that stand for a round hole: one, or with `twisted` the
/// five that layers take in turn, layer `n` taking the one at `n % 5`
/// (`create_polyholes`). Each is clockwise, with its flats on the circle
/// of `radius`: "cylinder(h = h, r = d / cos (180 / n), $fn = n)".
///
/// The corners are worked out in `f32` as the reference does. It also
/// adds the middle of the hole in `f32`, which in an object's own
/// coordinates costs it a unit or two; here coordinates are the bed's,
/// where that would cost up to sixteen, so the sum is made in `f64`.
pub fn create_polyholes(
    centre: Point2,
    radius: Coord,
    nozzle_diameter: Coord,
    twisted: bool,
    max_edges: i32,
) -> Vec<Polygon> {
    let edges = polyhole_edges(radius, nozzle_diameter, max_edges);
    let (count, rotation) = if twisted {
        (TWIST_STEPS, 2.0 * PI_F32 / (edges * TWIST_STEPS) as f32)
    } else {
        (1, 0.0)
    };
    let mut list = vec![Polygon::default(); count];
    let new_radius = radius as f32 / (std::f64::consts::PI / edges as f64).cos() as f32;
    for step in 0..count {
        // "alternate": the steps are dealt out so that one layer's
        // polygon is turned two fifths of a side from the last.
        let slot = if step % 2 == 0 {
            step / 2
        } else {
            count.div_ceil(2) + step / 2
        };
        let points = &mut list[slot].points;
        for edge in 0..edges {
            let angle = rotation * step as f32 + (PI_F32 * 2.0 * edge as f32) / edges as f32;
            points.push(Point2 {
                x: centre.x + f64::from(new_radius * angle.cos()).round() as Coord,
                y: centre.y + f64::from(new_radius * angle.sin()).round() as Coord,
            });
        }
        // `make_clockwise`: they were laid out counter-clockwise.
        points.reverse();
    }
    list
}

/// A round hole and where it is.
struct Found {
    hole: RoundHole,
    filament: usize,
    nozzle_diameter: f64,
    share: usize,
    region: usize,
    ring: usize,
}

/// Converts the round holes that run through the layers to polyholes, in
/// place, and says which rings were put in place of which.
///
/// Each layer is given as the shares its filaments have of it; a layer in
/// one filament is one share. Layers are taken to lie one on the next:
/// the reference lets a run go on while a layer's underside is at the top
/// of the last one that had the hole.
pub fn hole_to_polyholes(layers: &mut [Vec<Share<'_>>], polyholes: &Polyholes) -> Vec<Replaced> {
    // "get all circular holes for each layer"
    let mut found: Vec<Vec<Found>> = layers
        .par_iter()
        .map(|shares| {
            let mut round = Vec::new();
            for (share, own) in shares.iter().enumerate() {
                for (region, area) in own.regions.iter().enumerate() {
                    for (ring, hole) in area.holes.iter().enumerate() {
                        if let Some(hole) = round_hole(hole, polyholes.margin) {
                            round.push(Found {
                                hole,
                                filament: own.filament,
                                nozzle_diameter: own.nozzle_diameter,
                                share,
                                region,
                                ring,
                            });
                        }
                    }
                }
            }
            round
        })
        .collect();

    // "search & find hole that span at least X layers": a hole starts a
    // run, and takes with it from each layer above the first hole there
    // with its filament, middle and radius, until a layer has none.
    let mut replaced = Vec::new();
    for layer in 0..found.len() {
        let (upto, above) = found.split_at_mut(layer + 1);
        for first in &upto[layer] {
            let (id, filament, nozzle) = (first.hole, first.filament, first.nozzle_diameter);
            let mut run = vec![(layer, first.share, first.region, first.ring)];
            // The run ends at the first layer without the hole: the
            // reference goes on only while a layer's underside is at the
            // top of the last layer that had it.
            for (further, others) in above.iter_mut().enumerate() {
                let same = others.iter().position(|other| {
                    other.filament == filament
                        && distance(id.centre, other.hole.centre) < id.margin as f64
                        && (id.radius - other.hole.radius).abs() < id.margin as f32
                });
                let Some(at) = same else {
                    break;
                };
                let taken = others.remove(at);
                run.push((layer + 1 + further, taken.share, taken.region, taken.ring));
            }
            // "check if strait hole or first layer hole (cause of first
            // layer compensation)"
            if run.len() < MIN_LAYERS && !(run.len() == 1 && run[0].0 == 0) {
                continue;
            }
            // Not as the reference, which keeps its runs in a map by
            // middle, radius, filament and margin, and so drops a run
            // whose hole is, to the unit, the hole of a run it already
            // has: the same hole met again higher up, past layers where
            // it was not round, stays as drawn there. Here every run is
            // converted.
            let made = create_polyholes(
                id.centre,
                id.radius as Coord,
                scale(nozzle) as Coord,
                polyholes.twisted,
                polyholes.max_edges,
            );
            for (at, share, region, ring) in run {
                let hole = &mut layers[at][share].regions[region].holes[ring];
                let to = made[at % made.len()].clone();
                replaced.push(Replaced {
                    layer: at,
                    from: std::mem::replace(hole, to.clone()),
                    to,
                });
            }
        }
    }
    replaced
}

#[cfg(test)]
mod tests;
