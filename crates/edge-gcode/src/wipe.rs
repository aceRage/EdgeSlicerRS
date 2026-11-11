//! What is done at the two ends of a wall loop to hide its seam.
//!
//! - `wipe_before_external_loop`: an outer wall is come at from a little
//!   way inside the part. The travel ends there, the filament is fed
//!   forward again there, and the nozzle then moves out to where the
//!   wall starts without laying anything, so whatever oozes as the
//!   filament comes back is left inside (GCode.cpp `extrude_loop`, "Port
//!   of 'wipe inside before extruding an external perimeter' feature
//!   from super slicer").
//! - `wipe_on_loops`: an outer wall ends with a small move in toward the
//!   part before the nozzle leaves it or is wiped (GCode.cpp
//!   `extrude_loop`, "make the configured inward move before leaving the
//!   loop"; GCode/WipePathHelpers.cpp `wipe_on_loops_destination`).
//! - `wipe_inward`: the nozzle is wiped after an outer wall over the
//!   inner walls behind it, not back along the wall. The wipe is made
//!   at once and pulls no filament back; if the move that follows needs
//!   a retraction it is made where the wipe ends (GCode.cpp
//!   `extrude_loop`, "Execute the accepted path before another extrusion
//!   replaces it"; the path is `wipe/inward.rs`).
//!
//! None of these moves lays anything, and none is made that would end
//! off the bed.
//!
//! Where this is not the reference:
//!
//! - The walls the two inward options look for beside the seam are the
//!   island's loops. The reference also has the island's walls that are
//!   not loops (those of parts too thin for one) among them, which here
//!   are laid after the loops.
//! - Whether a wall starts, or an inner wall lies, over air is asked of
//!   the wall where it starts, or not at all: the reference goes by the
//!   kinds of the stretches the wall was cut into when it was made.
//! - An outer wall is taken to lie beside the point it is come at from
//!   (see [`Writer::wipe_before_wall`]); the reference measures, and
//!   at some corners its answer turns on a rounding.
//! - The inward wipe goes by the `wipe` of the filament in use. The
//!   reference makes it wherever any filament of the print has `wipe`
//!   on (GCode.cpp `set_extruders`, `m_wipe.enable`; `extrude_loop`
//!   then calls `m_wipe.wipe` without asking the extruder in use, as
//!   `GCode::retract` does ask), so a filament set not to wipe is
//!   wiped inward after its outer walls all the same.

mod inward;

use std::io::{self, Write as _};

use edge_extrusion::{ExtrusionLoop, Flow, Role};
use edge_geom::{Point2, UNITS_PER_MM, unscaled};

use crate::{Writer, distance, role_name};
use inward::{Line, SCALED_EPSILON, V};

/// The walls of an island laid so far, for the inward wipe to rest on
/// (WipePathHelpers.hpp `WipeInwardSupport`: "Printed prefix of one
/// region's perimeter sequence. Append each entity only after extrusion;
/// later walls and other regions cannot support an inward wipe.").
#[derive(Debug, Default)]
pub(crate) struct Printed {
    lines: Vec<Line>,
    inner: Vec<Line>,
}

impl Printed {
    fn add(&mut self, wall: &ExtrusionLoop) {
        let points = &wall.polygon.points;
        let lines = (0..points.len()).map(|at| Line {
            a: points[at],
            b: points[(at + 1) % points.len()],
        });
        if wall.role == Role::Perimeter {
            self.inner.extend(lines.clone());
        }
        self.lines.extend(lines);
    }
}

/// Point.cpp `Point::ccw_angle`: how far round, counter-clockwise, it is
/// from the way to `a` to the way to `b`, seen from `at`; more than
/// nothing and at most a whole turn.
fn ccw_angle(at: Point2, a: Point2, b: Point2) -> f64 {
    let angle = ((a.x - at.x) as f64).atan2((a.y - at.y) as f64)
        - ((b.x - at.x) as f64).atan2((b.y - at.y) as f64);
    if angle <= 0.0 {
        angle + std::f64::consts::TAU
    } else {
        angle
    }
}

/// Point.cpp `Point::rotate`: `point` turned by `angle` about `centre`.
fn rotate(point: Point2, angle: f64, centre: Point2) -> Point2 {
    let (sin, cos) = angle.sin_cos();
    let d = V::between(centre, point);
    Point2 {
        x: (centre.x as f64 + cos * d.x - sin * d.y + 0.5).floor() as i64,
        y: (centre.y as f64 + sin * d.x + cos * d.y + 0.5).floor() as i64,
    }
}

/// WipePathHelpers.cpp `sample_path_at_distance`: the point `target`
/// along a path from its start, or with `forward` off back from its
/// end. "For tiny loops the walk stops early and returns the last
/// reachable point."
pub(crate) fn sample_path_at_distance(path: &[Point2], forward: bool, target: f64) -> Point2 {
    let count = path.len();
    let mut remaining = target;
    let mut result = if forward { path[0] } else { path[count - 1] };
    for step in 0..count - 1 {
        if remaining <= 0.0 {
            break;
        }
        let (current, next) = if forward {
            (path[step], path[step + 1])
        } else {
            (path[count - 1 - step], path[count - 2 - step])
        };
        let segment = V::between(current, next);
        let length = segment.norm();
        if length < SCALED_EPSILON {
            continue;
        }
        if remaining <= length {
            return V::of(current)
                .plus(segment.times(remaining / length))
                .point();
        }
        remaining -= length;
        result = next;
    }
    result
}

/// WipePathHelpers.cpp `wipe_on_loops_destination`: where the nozzle
/// goes at the end of a loop laid along `path`, which starts at the
/// seam: a fifth of the nozzle's width from the seam, a third of the way
/// round from the loop's start toward its end, which is in toward the
/// part. None where the loop is too small or too sharp at its seam for
/// that to mean anything. `nozzle` is in coordinate units.
pub(crate) fn on_loops_destination(
    path: &[Point2],
    nozzle: f64,
    is_ccw: bool,
    is_hole: bool,
) -> Option<Point2> {
    if path.len() < 2 || nozzle <= 0.0 {
        return None;
    }
    // "clamp sample distance to L/4 so forward/backward samples cannot meet."
    let total: f64 = path
        .windows(2)
        .map(|pair| V::between(pair[0], pair[1]).norm())
        .sum();
    let sample = nozzle.min(total * 0.25);
    let mut a = sample_path_at_distance(path, true, sample);
    let mut b = sample_path_at_distance(path, false, sample);
    let seam = path[0];
    if a == b || a == seam || b == seam {
        return None;
    }
    let reverse_turn = is_hole == is_ccw;
    if reverse_turn {
        std::mem::swap(&mut a, &mut b);
    }
    let mut angle = ccw_angle(seam, a, b) / 3.0;
    // "reject degenerate angles near 0 or 2π."
    const ANGLE_EPSILON: f64 = 0.01;
    if !(ANGLE_EPSILON..=std::f64::consts::TAU / 3.0 - ANGLE_EPSILON).contains(&angle) {
        return None;
    }
    if reverse_turn {
        angle = -angle;
    }
    let point = sample_path_at_distance(path, true, (0.2 * nozzle).min(sample));
    Some(rotate(point, angle, seam))
}

/// GCode.cpp `extrude_loop`: where an outer wall laid along `path` is
/// come at from. The point is `2 * reach` from the wall's start, back
/// the way the wall ends and turned a third of the corner there in
/// toward the part. `reach` is in coordinate units: "the minimum between
/// half the nozzle diameter or half the line width for the upcoming
/// perimeter".
pub(crate) fn approach_point(
    path: &[Point2],
    reach: f64,
    is_ccw: bool,
    is_hole: bool,
) -> Option<Point2> {
    if path.len() < 2 {
        return None;
    }
    let current = path[0];
    let previous = path[1];
    let mut next = path[path.len() - 1];
    // "can happen if seam_gap is null"
    if next == current {
        next = path[path.len() - 2];
    }
    let turn_out = is_hole != is_ccw;
    let (a, b) = if turn_out {
        (previous, next)
    } else {
        (next, previous)
    };
    let mut angle = ccw_angle(current, a, b) / 3.0;
    if turn_out {
        angle = -angle;
    }
    let back = V::between(current, next);
    let length = back.norm();
    // A loop that ends where it starts and has nothing between gives no
    // way to go; the reference divides by the nothing.
    if length == 0.0 {
        return None;
    }
    let point = V::of(current)
        .plus(back.times(2.0 * reach / length))
        .point();
    Some(rotate(point, angle, current))
}

impl Writer<'_> {
    /// Whether a point is over the bed; anywhere is, of a bed that is
    /// not known.
    fn over_bed(&self, point: Point2) -> bool {
        self.settings
            .bed
            .is_none_or(|[min_x, min_y, max_x, max_y]| {
                let (x, y) = (unscaled(point.x), unscaled(point.y));
                x >= min_x && x <= max_x && y >= min_y && y <= max_y
            })
    }

    /// Whether a wall hangs clear of the layer beneath where it starts,
    /// and so starts as an overhang wall.
    fn starts_clear(&self, start: Point2) -> bool {
        self.clear_of()
            .is_some_and(|(below, by)| below.clear(unscaled(start.x), unscaled(start.y), by))
    }

    /// Comes at an outer wall from inside the part, if that is asked for
    /// (`wipe_before_external_loop`): to be called before the wall is
    /// laid along `path`, which starts at its seam. `own` is the
    /// island's walls, `turned` whether the wall is laid the other way
    /// round from how its points run, and `width` that of the line its
    /// first stretch is laid with, in mm.
    pub(crate) fn wipe_before_wall(
        &mut self,
        own: &[ExtrusionLoop],
        wall: &ExtrusionLoop,
        turned: bool,
        path: &[Point2],
        width: f64,
    ) -> io::Result<()> {
        let s = self.settings;
        // "If region perimeters size not greater than or equal to 2,
        // then skip the wipe inside move as we will extrude in mid air
        // as no neighbouring perimeter exists."
        if !s.wipe_before_external_loop
            || wall.role != Role::ExternalPerimeter
            || own.len() < 2
            || path.len() < 2
            || self.starts_clear(path[0])
        {
            return Ok(());
        }
        let is_ccw = wall.polygon.is_ccw() != turned;
        let reach = 0.5 * s.nozzle_diameter.min(width) * UNITS_PER_MM;
        let Some(point) = approach_point(path, reach, is_ccw, wall.is_hole) else {
            return Ok(());
        };
        // "If an internal perimeter exists, we should find 2 perimeters
        // touching the de-retraction point 1 - the currently printed
        // external perimeter and 2 - the neighbouring internal
        // perimeter." The wall itself is taken as found: the point is
        // made no further from where it starts than the nozzle is wide.
        // The reference looks for it like any other, strictly nearer
        // than that width, and where the part's side of the seam is a
        // corner of 270 degrees or more (any corner of a square hole)
        // the point is exactly that far off, so that whether the wall
        // is found there turns on how the point's coordinates were
        // rounded.
        let radius = s.nozzle_diameter * UNITS_PER_MM;
        let beside = own.iter().any(|other| {
            let points = &other.polygon.points;
            !std::ptr::eq(other, wall)
                && (0..points.len()).any(|at| {
                    let line = Line {
                        a: points[at],
                        b: points[(at + 1) % points.len()],
                    };
                    line.nearest(point).0 < radius * radius
                })
        });
        if !beside || !self.over_bed(point) {
            return Ok(());
        }
        self.travel(point, false, Some(wall.role))?;
        self.dry(path[0], wall.role, &wall.flow)
    }

    /// One move in a wall's kind that lays nothing, after bringing the
    /// nozzle down and the filament forward: the reference's
    /// `extrude_path` of a path made not to extrude. It goes at the
    /// speed the wall's kind is laid at, slowed with the layer as a wall
    /// is; as in the reference the speed is not that of a small loop
    /// (GCode.cpp `extrude_loop` hands on `speed`, not
    /// `speed_for_path`), and with no flow there is none to hold it to
    /// what the hot end can melt.
    fn dry(&mut self, to: Point2, role: Role, flow: &Flow) -> io::Result<()> {
        let s = self.settings;
        let Some(from) = self.state.position else {
            return Ok(());
        };
        let length = distance(from, to);
        if length == 0.0 {
            return Ok(());
        }
        self.prime()?;
        if self.state.role != Some(role) {
            if self.state.role.map(role_name) != Some(role_name(role)) {
                writeln!(self.out, ";TYPE:{}", role_name(role))?;
            }
            self.state.role = Some(role);
        }
        if self.state.width != flow.width {
            writeln!(self.out, ";WIDTH:{:.3}", flow.width)?;
            self.state.width = flow.width;
        }
        let first_layer = self.state.first_layer;
        let acceleration = self.motion(role, flow, first_layer).acceleration;
        self.accelerate(acceleration)?;
        if !self.extruding {
            self.extruding = true;
            self.timing.fixed = 0.0;
        }
        let wanted = if first_layer {
            s.initial_layer_speed
        } else {
            s.outer_wall_speed
        };
        let speed = if role == Role::ExternalPerimeter && s.dont_slow_down_outer_wall {
            self.timing.fixed += length / wanted;
            wanted
        } else {
            match self.timing.lines.last_mut() {
                Some(line) if line.1 == wanted => line.0 += length,
                _ => self.timing.lines.push((length, wanted)),
            }
            self.slowdown.apply(wanted)
        };
        self.feed(speed);
        let feed_word = self.feed_word();
        writeln!(
            self.out,
            "G1 X{:.3} Y{:.3}{feed_word}",
            unscaled(to.x),
            unscaled(to.y)
        )?;
        self.state.line_speed = wanted;
        self.state.summary.travel_mm += length;
        self.state.summary.seconds += length / speed;
        self.state.position = Some(to);
        Ok(())
    }

    /// Ends a wall loop that has just been laid: the move in toward the
    /// part (`wipe_on_loops`) and the wipe over the inner walls
    /// (`wipe_inward`), where they are asked for and the wall is an
    /// outer one. `turned` and `width` are as for
    /// [`Self::wipe_before_wall`]; `printed` is the island's walls laid
    /// before this one, which this one then joins.
    pub(crate) fn wipe_after_wall(
        &mut self,
        wall: &ExtrusionLoop,
        turned: bool,
        width: f64,
        printed: &mut Printed,
    ) -> io::Result<()> {
        let s = self.settings;
        // "Disabled or zero-length wipes need no support geometry."
        let inward = s.wipe_inward.filter(|distance| {
            s.wipe && s.wipe_distance * UNITS_PER_MM > SCALED_EPSILON && distance.resolve(1.0) > 0.0
        });
        if !s.wipe_on_loops && inward.is_none() {
            return Ok(());
        }
        // "Only external perimeters are eligible."
        if wall.role != Role::ExternalPerimeter {
            if inward.is_some() {
                printed.add(wall);
            }
            return Ok(());
        }
        // The wall as it was laid, from its seam to where the nozzle is.
        let mut path = self.state.wipe_path.clone();
        path.dedup();
        let Some(at) = self.state.position else {
            return Ok(());
        };
        let is_ccw = wall.polygon.is_ccw() != turned;
        let nozzle = s.nozzle_diameter * UNITS_PER_MM;

        // The wall's last stretch has to be an outer wall's, not one
        // laid over air.
        let destination = (s.wipe_on_loops
            && self.state.role == Some(Role::ExternalPerimeter)
            && s.wall_loops > 1
            && path.len() >= 2)
            .then(|| on_loops_destination(&path, nozzle, is_ccw, wall.is_hole))
            .flatten()
            .filter(|&point| self.over_bed(point));

        let wipe = inward
            .filter(|_| !printed.inner.is_empty() && path.len() >= 2)
            .and_then(|distance| {
                // "use the actual extrusion width from the path, not the
                // config value"
                let offset = distance.resolve(width).min(width) * UNITS_PER_MM;
                if offset <= SCALED_EPSILON {
                    return None;
                }
                let seam_start = path[0];
                let seam_end = path[path.len() - 1];
                let wipe_start = destination.unwrap_or(seam_end);
                let max_wipe_length = s.wipe_distance * UNITS_PER_MM;
                let support_distance = s.nozzle_diameter.max(width) * UNITS_PER_MM;
                // Every candidate has to have an inner wall this near
                // where the wipe starts (`wipe_path_support_score`).
                if inward::distance_from(&printed.inner, wipe_start)
                    > 3.0 * support_distance + 4.0 * SCALED_EPSILON
                {
                    return None;
                }
                // No line further off than this is ever the nearest to
                // anything the wipe is tested at, so the rest are left
                // out of the search.
                let reach = 2.0
                    * (V::between(wipe_start, seam_start).norm()
                        + V::between(wipe_start, seam_end).norm()
                        + max_wipe_length)
                    + 12.0 * support_distance;
                let near = |lines: &[Line]| -> Vec<Line> {
                    lines
                        .iter()
                        .filter(|line| line.nearest(wipe_start).0 <= reach * reach)
                        .copied()
                        .collect()
                };
                inward::offset_wipe_path_toward_support(
                    &path,
                    seam_start,
                    seam_end,
                    wipe_start,
                    inward::offset_direction(is_ccw, wall.is_hole),
                    offset,
                    max_wipe_length,
                    &near(&printed.inner),
                    &near(&printed.lines),
                    &near(&inward::lines_of(&path)),
                    support_distance,
                )
            })
            .filter(|path| path[1..].iter().all(|&point| self.over_bed(point)));
        if inward.is_some() {
            printed.add(wall);
        }

        if let Some(to) = destination {
            // At whatever speed the wall ended at (GCodeWriter.cpp
            // `extrude_to_xy`, which says none).
            let length = distance(at, to);
            let speed = self.state.feedrate.abs() / 60.0;
            if length > 0.0 && speed > 0.0 {
                let up = self.up_to_layer();
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3}{up}",
                    unscaled(to.x),
                    unscaled(to.y)
                )?;
                self.state.summary.travel_mm += length;
                self.spend(length / speed);
                self.state.position = Some(to);
            }
        }
        if let Some(path) = wipe
            && let Some(mut from) = self.state.position
        {
            // GCode.cpp `Wipe::wipe` with nothing to pull back: the
            // moves alone, at the wipe's speed.
            let speed = s.wipe_speed.unwrap_or(self.state.line_speed).max(10.0);
            self.state.feedrate = (speed * 60.0).round();
            writeln!(self.out, ";WIPE_START\nG1 F{:.0}", self.state.feedrate)?;
            for &to in &path[1..] {
                let length = distance(from, to);
                if length == 0.0 {
                    continue;
                }
                let up = self.up_to_layer();
                writeln!(
                    self.out,
                    "G1 X{:.3} Y{:.3}{up}",
                    unscaled(to.x),
                    unscaled(to.y)
                )?;
                self.state.summary.travel_mm += length;
                self.spend(length / speed);
                self.state.position = Some(to);
                from = to;
            }
            writeln!(self.out, ";WIPE_END")?;
            // "prevent wiping again on same path"
            self.state.wipe_path.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "wipe_tests.rs"]
mod tests;
