//! Scarf seams: a wall loop joined on a slope, not end to end.
//!
//! A loop joined this way is laid whole, with no gap left at its seam.
//! Its first stretch climbs from the top of the layer beneath to the
//! layer's own height, laid thinner the lower it is, and when the loop
//! comes round to its start it carries on over that stretch at full
//! height, laid thinner the further it goes, so the two halves of the
//! joint make up one line between them (GCode.cpp `extrude_loop`, "Create
//! seam slope"; ExtrusionEntity.cpp `ExtrusionLoopSloped`; GCode.cpp
//! `_extrude`, "Sloped extrusion").
//!
//! Which loops are joined so is `seam_slope_type`: none, those round the
//! part, or those round holes as well; outer walls, and inner ones with
//! `seam_slope_inner_walls`; never on the first layer or in spiral vase
//! mode, nor a loop that starts over air (the reference goes by the
//! kind of a loop's first stretch, and that one starts with an overhang
//! wall). With `seam_slope_conditional` only loops with no sharp corner
//! (`scarf_angle_threshold`) whose seam rests on the layer beneath
//! (`scarf_overhang_threshold`) are. The walls of a circle the auto
//! circle compensation resized are, whatever any of those say, where
//! `apply_scarf_seam_on_circles` is on.
//!
//! Where this is not the reference:
//!
//! - A scarf of no length (`seam_slope_min_length` at 0) is a plain
//!   seam. The reference lays such a loop whole, with neither a slope
//!   nor the seam gap (GCode.cpp `extrude_loop`: `clip_length` is zero
//!   once `enable_seam_slope` is set, whatever the slope's length).
//! - An inner wall's slope starts two fifths of its length on from the
//!   seam whatever the seam gap is. The reference tests the wrong
//!   length before it clips (ExtrusionEntity.cpp `clip_front`: `while
//!   (distance > 0 ...)` where `clip_dist` is meant), so with
//!   `seam_gap` at 0 nothing is clipped. (It also asks the loop's kind
//!   of what is left of the loop after the slope, `this->role()`, and
//!   so takes an inner wall whose slope is the whole loop for no wall
//!   at all and starts it two seam gaps on, as an outer wall. That is
//!   kept, on purpose here: two fifths of a loop left out of its first
//!   time round would leave the whole wall a fifth short of plastic.
//!   Where the loop goes on flat past its slope the wall's own kind is
//!   asked, not that of the flat part's first stretch, which the
//!   reference takes for no inner wall where it hangs over air.)
//! - `scarf_joint_speed` holds the slope's speed on a small loop too,
//!   a percentage being of the wall's own speed there as anywhere.
//!   The reference only looks at it where no speed was handed down
//!   (GCode.cpp `_extrude`, `if (speed == -1)`), and a small loop's is.
//! - The share of the wall's width a seam may hang out by before it
//!   gets no scarf is of the wall's own width. The reference takes the
//!   width as set (GCode.cpp `extrude_loop`, `_line_width`), which is 0
//!   where it is left to follow the nozzle.
//! - The nozzle goes down to the foot of the slope after the travel
//!   there, as its own move; the reference makes the last leg of an
//!   unretracted travel the way down.

use std::io;

use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionLoop, Flow, Role};
use edge_geom::{Point2, UNITS_PER_MM, unscaled};

use crate::{Layer, Writer, arc, overhang, seam};

/// libslic3r.h `EPSILON`.
const EPSILON: f64 = 1e-4;
/// ExtrusionEntity.cpp `seam_path_tolerance`, in coordinate units: "An
/// eighth of a common line width: the path moves by less than that
/// where a vertex is dropped."
const SEAM_PATH_TOLERANCE: f64 = 0.05 * UNITS_PER_MM;
/// ExtrusionEntity.cpp `slope_inner_outer_wall_gap`: how far on from the
/// seam an inner wall's slope starts, as a share of the slope's length.
const INNER_WALL_GAP: f64 = 0.4;

/// Which loops are joined with a scarf (`seam_slope_type`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScarfSeam {
    None,
    /// Those round the part, not those round its holes.
    External,
    All,
}

/// What the scarf seam settings come to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scarf {
    pub kind: ScarfSeam,
    /// Only loops without a sharp corner, whose seam does not hang out
    /// (`seam_slope_conditional`).
    pub conditional: bool,
    /// A corner sharper than this many degrees is a sharp one
    /// (`scarf_angle_threshold`; a straight run is 180).
    pub angle_threshold: f64,
    /// The seam may be this share of the wall's width short of resting
    /// on the layer beneath (`scarf_overhang_threshold`); zero for no
    /// such limit.
    pub overhang_threshold: f64,
    /// The speed of the joint, where that is less than the wall's
    /// (`scarf_joint_speed`); a percentage is of the wall's speed.
    pub joint_speed: FloatOrPercent,
    /// `scarf_joint_flow_ratio`.
    pub joint_flow_ratio: f64,
    /// How high above the layer beneath the slope starts, in mm or as a
    /// percentage of the layer's height (`seam_slope_start_height`).
    pub start_height: FloatOrPercent,
    /// `seam_slope_entire_loop`: the slope is as long as the loop.
    pub entire_loop: bool,
    /// The slope's length in mm (`seam_slope_min_length`); no longer
    /// than the loop.
    pub min_length: f64,
    /// The slope is laid in at least this many moves (`seam_slope_steps`).
    pub steps: usize,
    /// `seam_slope_inner_walls`.
    pub inner_walls: bool,
    /// `apply_scarf_seam_on_circles`.
    pub on_circles: bool,
}

impl Default for Scarf {
    /// PrintConfig.cpp.
    fn default() -> Self {
        Self {
            kind: ScarfSeam::None,
            conditional: false,
            angle_threshold: 155.0,
            overhang_threshold: 0.4,
            joint_speed: FloatOrPercent::Percent(100.0),
            joint_flow_ratio: 1.0,
            start_height: FloatOrPercent::Value(0.0),
            entire_loop: false,
            min_length: 20.0,
            steps: 10,
            inner_walls: false,
            on_circles: true,
        }
    }
}

impl Scarf {
    /// Reads the settings. What cannot be followed is added to `notes`.
    pub(crate) fn read(settings: &Settings, notes: &mut Vec<String>) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let bad = |key: &str, expected: &'static str| ConfigError::BadValue {
            key: key.into(),
            found: settings.text(key).unwrap_or_default().to_owned(),
            expected,
        };
        let flag = |key: &str, default: bool| settings.flag(key).map(|set| set.unwrap_or(default));
        // GCode.cpp `extrude_loop`: "!m_config.spiral_mode".
        let spiral = flag("spiral_mode", false)?;
        let kind = match settings.text("seam_slope_type") {
            _ if spiral => ScarfSeam::None,
            None | Some("none") => ScarfSeam::None,
            Some("external") => ScarfSeam::External,
            Some("all") => ScarfSeam::All,
            Some(_) => return Err(bad("seam_slope_type", "none, external or all")),
        };
        let on_circles = !spiral && flag("apply_scarf_seam_on_circles", defaults.on_circles)?;
        let angle_threshold = settings
            .float("scarf_angle_threshold")?
            .unwrap_or(defaults.angle_threshold);
        if !(0.0..=180.0).contains(&angle_threshold) {
            return Err(bad("scarf_angle_threshold", "an angle from 0 to 180"));
        }
        let overhang_threshold = match settings.text("scarf_overhang_threshold") {
            None => defaults.overhang_threshold,
            Some(text) => text
                .trim()
                .trim_end_matches('%')
                .parse::<f64>()
                .ok()
                .filter(|percent| *percent >= 0.0)
                .map(|percent| percent / 100.0)
                .ok_or_else(|| bad("scarf_overhang_threshold", "a percentage"))?,
        };
        let mut joint_speed = settings
            .float_or_percent("scarf_joint_speed")?
            .unwrap_or(defaults.joint_speed);
        if joint_speed.resolve(1.0) < 0.0 {
            return Err(bad("scarf_joint_speed", "a speed or a percentage"));
        }
        // The reference prints a joint with no speed as fast as the hot
        // end can melt: a speed of nothing is taken for one not set
        // (GCode.cpp `_extrude`, "if not set the speed"). That is not
        // what anyone asking for a slow joint means.
        if joint_speed.resolve(1.0) == 0.0 {
            if kind != ScarfSeam::None {
                notes.push(
                    "scarf_joint_speed \"0\": scarf joints are laid at their walls' speed".into(),
                );
            }
            joint_speed = defaults.joint_speed;
        }
        let joint_flow_ratio = settings
            .float("scarf_joint_flow_ratio")?
            .unwrap_or(defaults.joint_flow_ratio);
        if !(joint_flow_ratio > 0.0 && joint_flow_ratio <= 2.0) {
            return Err(bad(
                "scarf_joint_flow_ratio",
                "a ratio above 0 and no more than 2",
            ));
        }
        let start_height = settings
            .float_or_percent("seam_slope_start_height")?
            .unwrap_or(defaults.start_height);
        if start_height.resolve(1.0) < 0.0 {
            return Err(bad(
                "seam_slope_start_height",
                "a height of 0 or more, or a percentage",
            ));
        }
        let min_length = settings
            .float("seam_slope_min_length")?
            .unwrap_or(defaults.min_length);
        if min_length.is_nan() || min_length < 0.0 {
            return Err(bad("seam_slope_min_length", "a length of 0 or more"));
        }
        let steps = match settings.integer("seam_slope_steps")? {
            None => defaults.steps,
            Some(steps) if steps >= 1 => steps as usize,
            Some(_) => return Err(bad("seam_slope_steps", "1 or more")),
        };
        Ok(Self {
            kind,
            conditional: flag("seam_slope_conditional", defaults.conditional)?,
            angle_threshold,
            overhang_threshold,
            joint_speed,
            joint_flow_ratio,
            start_height,
            entire_loop: flag("seam_slope_entire_loop", defaults.entire_loop)?,
            min_length,
            steps,
            inner_walls: flag("seam_slope_inner_walls", defaults.inner_walls)?,
            on_circles,
        })
    }
}

/// Distance between two points in coordinate units.
fn span(a: Point2, b: Point2) -> f64 {
    ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
}

/// Length of a path through `points`, in coordinate units.
fn length(points: &[Point2]) -> f64 {
    points.windows(2).map(|pair| span(pair[0], pair[1])).sum()
}

/// A stretch of a loop laid one way: one of the paths the reference
/// makes a loop of (one for a loop of one width, one for each width of a
/// loop whose width changes).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub(crate) line: arc::Fitted,
    pub(crate) flow: Flow,
}

/// A stretch of a scarf (ExtrusionEntity.hpp `ExtrusionPathSloped`):
/// from its first point to its last it goes evenly from one height and
/// flow to another, each a share of the layer's own.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stretch {
    pub(crate) points: Vec<Point2>,
    pub(crate) flow: Flow,
    /// The share of the layer's height the nozzle is at, and of the
    /// line's flow that is laid, at the first point and at the last.
    pub(crate) from: (f64, f64),
    pub(crate) to: (f64, f64),
}

/// A loop joined with a scarf (ExtrusionEntity.hpp
/// `ExtrusionLoopSloped`): laid in this order.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Sloped {
    /// The way up, from the seam.
    pub(crate) starts: Vec<Stretch>,
    /// The rest of the loop, back round to the seam.
    pub(crate) flat: Vec<Run>,
    /// Over the way up again, thinning out.
    pub(crate) ends: Vec<Stretch>,
}

/// ExtrusionEntity.cpp `trim_seam_ends`: "The seam is inserted into the
/// loop unless a vertex lies within the G-code resolution of it, so a
/// loop can begin and end with a segment of a few micrometres. A plain
/// loop stops there anyway; a scarf extrudes through both ends, and the
/// planner nearly halts on a block that short. Drop the vertex next to
/// the seam point instead, so the loop still starts and ends at the
/// seam. A trimmed path loses its arc fitting; its geometry is
/// unchanged, it just prints as line segments."
fn trim_seam_ends(runs: &mut Vec<Run>, tolerance: f64) {
    let straightened = |run: &mut Run, change: &dyn Fn(&mut Vec<Point2>)| {
        let mut points = std::mem::take(&mut run.line.points);
        change(&mut points);
        run.line = arc::Fitted::straight(points);
    };
    while let Some(points) = runs.first().map(|run| &run.line.points) {
        if points.len() < 2 || span(points[0], points[1]) >= tolerance {
            break;
        }
        if points.len() > 2 {
            straightened(&mut runs[0], &|points| {
                points.remove(1);
            });
        } else if runs.len() > 1 {
            let seam = points[0];
            runs.remove(0);
            straightened(&mut runs[0], &|points| points[0] = seam);
        } else {
            break;
        }
    }
    while let Some(points) = runs.last().map(|run| &run.line.points) {
        let (count, last) = (points.len(), runs.len() - 1);
        if count < 2 || span(points[count - 2], points[count - 1]) >= tolerance {
            break;
        }
        if count > 2 {
            straightened(&mut runs[last], &|points| {
                points.remove(count - 2);
            });
        } else if runs.len() > 1 {
            let seam = points[count - 1];
            runs.pop();
            straightened(&mut runs[last - 1], &|points| {
                let end = points.len() - 1;
                points[end] = seam;
            });
        } else {
            break;
        }
    }
}

/// ExtrusionEntity.cpp `split_at_slope_end`: "Split `polyline` where the
/// scarf ramp ends. A remainder shorter than half a slope step before
/// the next vertex would be a flat stub that makes the motion planner
/// slow down at the end of the ramp. Where that vertex lies within
/// `seam_path_tolerance` of the straight line from the exact ramp end to
/// the vertex after it, the flat path skips it: the ramp keeps its exact
/// length, so it ends at the same place on every layer however the
/// loop's vertices fall there. At a corner, or when the vertex ends the
/// path, the ramp is extended to the vertex instead." Lengths are in
/// coordinate units. Gives the slope's points and what is left flat.
fn split_at_slope_end(
    line: &arc::Fitted,
    slope_length: f64,
    max_segment: f64,
) -> (Vec<Point2>, arc::Fitted) {
    let points = &line.points;
    let snap = (0.5 * max_segment).min(UNITS_PER_MM);
    let mut done = 0.0;
    for (index, pair) in points.windows(2).enumerate() {
        let whole = span(pair[0], pair[1]);
        let end = done + whole;
        if end >= slope_length {
            if end - slope_length < snap {
                let after = index + 2;
                if slope_length >= arc::tolerance(EPSILON) && after < points.len() {
                    let share = (slope_length - done) / whole;
                    let (ax, ay) = (
                        pair[0].x as f64 + share * (pair[1].x - pair[0].x) as f64,
                        pair[0].y as f64 + share * (pair[1].y - pair[0].y) as f64,
                    );
                    let (bx, by) = (points[after].x as f64, points[after].y as f64);
                    let (px, py) = (pair[1].x as f64, pair[1].y as f64);
                    let chord = (bx - ax).powi(2) + (by - ay).powi(2);
                    let t = if chord > 0.0 {
                        (((px - ax) * (bx - ax) + (py - ay) * (by - ay)) / chord).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let off = (px - ax - t * (bx - ax)).hypot(py - ay - t * (by - ay));
                    if off <= SEAM_PATH_TOLERANCE
                        && let Some((slope, _)) = line.split_at_length(slope_length)
                    {
                        let (_, mut flat) = line.split_at_index(after);
                        if let Some(&end) = slope.points.last() {
                            flat.push_front(end);
                        }
                        return (slope.points, flat);
                    }
                }
                let (slope, flat) = line.split_at_index(index + 1);
                return (slope.points, flat);
            }
            break;
        }
        done = end;
    }
    match line.split_at_length(slope_length) {
        Some((slope, flat)) => (slope.points, flat),
        // Not as long as the slope after all: all of it is slope.
        None => (
            points.clone(),
            arc::Fitted::straight(points.last().copied().into_iter().collect()),
        ),
    }
}

/// A straight path cut into moves no longer than `max_segment`
/// (ExtrusionEntity.cpp `add_slop`, "Recursively split the line into
/// half until no longer than `slope_max_segment_length`").
fn detailed(points: &[Point2], max_segment: f64) -> Vec<Point2> {
    fn halve(a: Point2, b: Point2, max_segment: f64, out: &mut Vec<Point2>) {
        // A line of a unit or two has no middle to cut it at.
        if span(a, b) <= max_segment.max(2.0) {
            out.push(b);
            return;
        }
        // Line.hpp `midpoint`.
        let middle = Point2 {
            x: (a.x + b.x) / 2,
            y: (a.y + b.y) / 2,
        };
        halve(a, middle, max_segment, out);
        halve(middle, b, max_segment, out);
    }
    let mut out = Vec::with_capacity(points.len());
    out.extend(points.first());
    for pair in points.windows(2) {
        halve(pair[0], pair[1], max_segment, &mut out);
    }
    out
}

/// Polyline.cpp `Polyline::clip_end` for a path with no arcs: shortens
/// it by `distance` coordinate units from its end. A path no longer
/// than that is left empty.
fn clip_end(points: &mut Vec<Point2>, mut distance: f64) {
    while distance > 0.0 {
        let Some(last) = points.pop() else {
            return;
        };
        let Some(&before) = points.last() else {
            return;
        };
        let (vx, vy) = ((before.x - last.x) as f64, (before.y - last.y) as f64);
        let lsqr = vx * vx + vy * vy;
        if lsqr > distance * distance {
            let share = distance / lsqr.sqrt();
            points.push(Point2 {
                x: (last.x as f64 + vx * share) as i64,
                y: (last.y as f64 + vy * share) as i64,
            });
            return;
        }
        distance -= lsqr.sqrt();
    }
}

/// The first `length` coordinate units of a path with no arcs
/// (Polyline.cpp `Polyline::split_at_length`, its first half).
fn head(points: &[Point2], length: f64) -> Vec<Point2> {
    match arc::Fitted::straight(points.to_vec()).split_at_length(length) {
        Some((head, _)) => head.points,
        None => points.to_vec(),
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a * (1.0 - t) + b * t
}

/// ExtrusionEntity.cpp `ExtrusionLoopSloped::ExtrusionLoopSloped` and
/// `clip_slope`. `runs` is the loop once round from its seam, whole;
/// `gap` the seam gap and `max_segment` the longest move of the slope,
/// both in coordinate units; `slope_mm` the slope's length in mm;
/// `start_ratio` the share of the layer's height it starts at; `inner`
/// whether the loop is an inner wall.
pub(crate) fn sloped(
    mut runs: Vec<Run>,
    gap: f64,
    slope_mm: f64,
    max_segment: f64,
    start_ratio: f64,
    inner: bool,
) -> Sloped {
    trim_seam_ends(&mut runs, SEAM_PATH_TOLERANCE);
    // "The caller measured the loop before the trim; a scarf that
    // covers the whole loop must still end at 1."
    let trimmed: f64 = runs
        .iter()
        .map(|run| length(&run.line.points) / UNITS_PER_MM)
        .sum();
    let slope_mm = slope_mm.min(trimmed);
    let mut out = Sloped::default();
    let add = |out: &mut Sloped, flow: Flow, points: &[Point2], begin: f64, end: f64| {
        if points.len() < 2 {
            return;
        }
        let mut points = detailed(points, max_segment);
        out.starts.push(Stretch {
            points: points.clone(),
            flow,
            from: (begin, begin),
            to: (end, end),
        });
        let mut end = end;
        if (end - 1.0).abs() < EPSILON && gap > 0.0 {
            // "Remove the segments that has no extrusion"
            let whole = length(&points);
            if whole > gap {
                points = head(&points, whole - gap);
                end = lerp(begin, end, (whole - gap) / whole);
            } else {
                points.clear();
            }
        }
        if points.len() >= 2 {
            out.ends.push(Stretch {
                points,
                flow,
                from: (1.0, 1.0 - begin),
                to: (1.0, 1.0 - end),
            });
        }
    };
    let mut remaining = slope_mm;
    let mut start = start_ratio;
    let mut rest = runs.into_iter();
    while remaining > 0.0 {
        let Some(run) = rest.next() else {
            break;
        };
        let run_mm = length(&run.line.points) / UNITS_PER_MM;
        if run_mm > remaining {
            let (slope, flat) =
                split_at_slope_end(&run.line, remaining * UNITS_PER_MM, max_segment);
            add(&mut out, run.flow, &slope, start, 1.0);
            start = 1.0;
            if flat.points.len() > 1 {
                out.flat.push(Run {
                    line: flat,
                    flow: run.flow,
                });
            }
            remaining = 0.0;
        } else {
            remaining -= run_mm;
            let end = lerp(1.0, start_ratio, remaining / slope_mm);
            add(&mut out, run.flow, &run.line.points, start, end);
            start = end;
        }
    }
    out.flat.extend(rest);

    // `clip_slope`: the end of the way down stops a seam gap short...
    let mut left = gap;
    while left > 0.0 {
        let Some(last) = out.ends.last_mut() else {
            break;
        };
        let whole = length(&last.points);
        if whole <= left {
            out.ends.pop();
            left -= whole;
        } else {
            clip_end(&mut last.points, left);
            break;
        }
    }
    // ...and the way up starts two seam gaps on, or for an inner wall
    // two fifths of the slope on, so its joint is not beside the outer
    // wall's; not where the slope is the whole loop. What is left of
    // the stretch cut into still climbs from where the whole of it
    // would have. (See the top of this file for what the reference
    // does here.)
    let mut left = if inner && !out.flat.is_empty() {
        out.starts
            .iter()
            .map(|start| length(&start.points))
            .sum::<f64>()
            * INNER_WALL_GAP
    } else {
        2.0 * gap
    };
    while left > 0.0 {
        let Some(first) = out.starts.first_mut() else {
            break;
        };
        let whole = length(&first.points);
        if whole <= left {
            out.starts.remove(0);
            left -= whole;
        } else {
            first.points.reverse();
            clip_end(&mut first.points, left);
            first.points.reverse();
            break;
        }
    }
    out
}

/// ExtrusionEntity.cpp `ExtrusionLoop::is_smooth`: whether a loop has no
/// corner sharper than `threshold` radians, each measured between
/// points at least `arm` mm either side of it. `points` is the loop
/// once round from its seam, its first point not given again.
pub(crate) fn is_smooth(points: &[Point2], threshold: f64, arm: f64) -> bool {
    let count = points.len();
    if count < 3 {
        return false;
    }
    // In single precision, as the reference keeps them.
    let mm = |a: Point2, b: Point2| unscaled(a.x - b.x).hypot(unscaled(a.y - b.y)) as f32;
    let mut lengths: Vec<f32> = points.windows(2).map(|pair| mm(pair[0], pair[1])).collect();
    lengths.push(mm(points[0], points[count - 1]).max(0.1));
    let arm = arm as f32;
    let previous = |index: usize| if index == 0 { count - 1 } else { index - 1 };
    let next = |index: usize| if index + 1 == count { 0 } else { index + 1 };
    let (mut before, mut after) = (0usize, 0usize);
    let (mut to_before, mut to_after) = (0f32, 0f32);
    // "push idx_prev far enough back as initialization"
    while to_before < arm {
        before = previous(before);
        to_before += lengths[before];
    }
    for (at, length) in lengths.iter().enumerate() {
        // "pull idx_prev to current as much as possible, while
        // respecting the min_arm_length"
        while to_before - lengths[before] > arm {
            to_before -= lengths[before];
            before = next(before);
        }
        // "push idx_next forward as far as needed"
        while to_after < arm {
            to_after += lengths[after];
            after = next(after);
        }
        let (p0, p1, p2) = (points[before], points[at], points[after]);
        let (ax, ay) = ((p0.x - p1.x) as f64, (p0.y - p1.y) as f64);
        let (bx, by) = ((p2.x - p1.x) as f64, (p2.y - p1.y) as f64);
        let angle = (ax * by - ay * bx).atan2(ax * bx + ay * by);
        let sharp = if angle > 0.0 {
            angle < threshold
        } else {
            angle > -threshold
        };
        if sharp {
            return false;
        }
        to_before += length;
        to_after -= length;
    }
    true
}

/// Where the nozzle is along a stretch of a scarf.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Slope {
    from: (f64, f64),
    to: (f64, f64),
    /// The stretch's length and how much of it is laid, in mm.
    total: f64,
    done: f64,
    /// The layer's thickness in mm.
    height: f64,
}

impl Slope {
    /// How far through the stretch the nozzle is, from 0 to 1.
    fn along(&self) -> f64 {
        // Whatever the sum of its moves comes to, the stretch ends
        // exactly where it was to.
        if self.total <= 0.0 || self.done >= self.total - 1e-6 {
            1.0
        } else {
            (self.done / self.total).clamp(0.0, 1.0)
        }
    }

    /// How far below the layer's own height the nozzle is, in mm: never
    /// more than the layer is thick, so never under the layer.
    fn drop(&self) -> f64 {
        let share = lerp(self.from.0, self.to.0, self.along()).clamp(0.0, 1.0);
        (1.0 - share) * self.height
    }

    /// The share of the line's flow laid where the nozzle is.
    fn flow(&self) -> f64 {
        lerp(self.from.1, self.to.1, self.along()).clamp(0.0, 1.0)
    }
}

/// One move of a scarf, as [`Writer::lay`] writes it.
pub(crate) struct Sloping {
    /// The height the move ends at, in mm above the bed.
    pub(crate) z: f64,
    /// How far the nozzle goes, the climb counted in, in mm.
    pub(crate) length: f64,
    /// What the move lays, as a share of the line's flow.
    pub(crate) flow: f64,
}

impl Writer<'_> {
    /// The height the nozzle extrudes at where it is: the layer's, or
    /// lower on the way up a scarf.
    pub(crate) fn laying_height(&self) -> f64 {
        match &self.state.slope {
            // Not below the bed, whatever the layer is said to be.
            Some(slope) => (self.state.z - slope.drop()).max(0.0),
            None => self.state.z,
        }
    }

    /// Takes the nozzle `length` mm on along the scarf it is on, if it
    /// is on one, and says what the move is. As the reference has it,
    /// a move is laid at the flow its far end is to have (GCode.cpp
    /// `_extrude`, "Sloped extrusion": `interpolate(path_length /
    /// total_length)`): too much on the way up by half a move's share
    /// and too little by as much on the way down, which lies on it.
    pub(crate) fn slope_on(&mut self, length: f64) -> Option<Sloping> {
        let from = self.laying_height();
        let slope = self.state.slope.as_mut()?;
        slope.done += length;
        let flow = slope.flow() * self.settings.scarf.joint_flow_ratio;
        let z = self.laying_height();
        Some(Sloping {
            z,
            length: length.hypot(z - from),
            flow,
        })
    }

    /// A wall's speed on a scarf: no more than `scarf_joint_speed`, a
    /// percentage being of the wall's own speed, `own` (its kind's, or
    /// the compensation speed for a compensated circle's wall), as the
    /// option's own words have it, and not of the speed handed down,
    /// which on a small loop is `small_perimeter_speed` already: at
    /// 50%, a wall of 100 mm/s that is a small loop at 60 has its joint
    /// at 50, not at 30. A joint speed of nothing or less is no speed
    /// of its own. Elsewhere the speed as given.
    ///
    /// This is the rule the reference is being mended to
    /// (aceRage/EdgeSlicer#958, GCode.cpp `_extrude`,
    /// `cap_scarf_joint_speed`). The pinned reference only looks at the
    /// joint's speed where no speed was handed down (`if (speed ==
    /// -1)`), so there a small loop's joint goes at the small loop's
    /// speed whatever was asked.
    pub(crate) fn scarf_speed(&self, speed: f64, own: f64) -> f64 {
        match self.state.slope {
            Some(_) => {
                let joint = self.settings.scarf.joint_speed.resolve(own);
                if joint > 0.0 && joint < speed {
                    joint
                } else {
                    speed
                }
            }
            None => speed,
        }
    }

    /// What a line's flow is taken to be on a scarf where the hot end's
    /// limit is worked out, as a share of its own
    /// (`scarf_joint_flow_ratio`: GCode.cpp `_extrude`, "else if
    /// (sloped)").
    pub(crate) fn scarf_flow(&self) -> f64 {
        match self.state.slope {
            Some(_) => self.settings.scarf.joint_flow_ratio,
            None => 1.0,
        }
    }

    /// Whether the settings ask for a scarf on this wall, before its
    /// shape is looked at (GCode.cpp `extrude_loop`). `first` and
    /// `second` are where it starts and the next point on.
    fn scarf_asked(&self, wall: &ExtrusionLoop, first: Point2, second: Point2) -> bool {
        let scarf = &self.settings.scarf;
        let kind = match scarf.kind {
            ScarfSeam::None => false,
            ScarfSeam::External => !wall.is_hole,
            ScarfSeam::All => true,
        };
        let role = wall.role == Role::ExternalPerimeter
            || (wall.role == Role::Perimeter && scarf.inner_walls);
        // The reference goes by the kind of the loop's first path, and
        // a wall that starts over air starts with an overhang wall.
        kind && role && !self.state.first_layer && !self.over_air(first, second, wall.flow.width)
    }

    /// Whether the line from `first` to `second` starts clear of the
    /// layer beneath, as the walls are judged when they are laid.
    fn over_air(&self, first: Point2, second: Point2, width: f64) -> bool {
        let s = self.settings;
        let (Some(below), false, true) = (
            self.below.as_ref(),
            self.state.first_layer,
            s.detect_overhang_wall,
        ) else {
            return false;
        };
        overhang::Judge {
            below,
            width,
            clear_beyond: Some(0.5 * s.nozzle_diameter),
            speed: &|_| 0.0,
            fan_at: None,
        }
        .pieces(&[first, second])
        .first()
        .is_some_and(|piece| piece.clear)
    }

    /// Whether a seam rests on the layer beneath well enough for a
    /// scarf (GCode.cpp `extrude_loop`, `seam_overhang`;
    /// SeamPlacer.cpp `calculate_overhangs_and_layer_embedding`,
    /// `unsupported_dist`): the outer wall's line there may be short of
    /// the outline beneath by no more than `scarf_overhang_threshold`
    /// of the wall's width, counted from four tenths of the outer
    /// wall's width behind its middle. `unsupported` is that measure at
    /// the seam the wall starts at or beside, and `width` the width of
    /// the wall asked about.
    fn seam_rests(&self, unsupported: Option<f32>, width: f64) -> bool {
        let threshold = self.settings.scarf.overhang_threshold;
        if threshold <= 0.0 {
            return true;
        }
        unsupported.is_some_and(|short| f64::from(short) < threshold * width)
    }

    /// The loop as the reference would hold it: once round from its
    /// seam, whole, in one path or, where its width changes, one for
    /// each width, each with whatever arcs fit it. `path` is the loop
    /// opened at the seam and turned the way it is laid; `edge` and
    /// `start` say where on `wall` that is.
    fn scarf_runs(
        &self,
        wall: &ExtrusionLoop,
        turned: bool,
        path: &[Point2],
        (edge, start): (usize, Point2),
    ) -> Vec<Run> {
        let s = self.settings;
        if wall.widths.is_empty() {
            // As `fit_loop`, with nothing clipped from its end.
            let fitted = match self.fits.get(&(crate::address(wall), turned)) {
                Some(fitted) => Some(fitted.clone()),
                None => crate::fit_closed(s, &wall.polygon.points, turned, wall.role),
            };
            let line = match fitted {
                Some(mut fitted) => {
                    fitted.start_at(path[0]);
                    fitted
                }
                None => arc::Fitted::straight(path.to_vec()),
            };
            return vec![Run {
                line,
                flow: wall.flow,
            }];
        }
        // As `walls` cuts a wall whose width changes into pieces.
        let widths: Vec<f64> = wall.widths.iter().map(|&width| width as f64).collect();
        let mut widths = seam::open_widths_at(&wall.polygon.points, &widths, edge, start);
        if turned {
            widths.reverse();
        }
        let line = edge_medial::ThickPolyline {
            widths: widths
                .windows(2)
                .flat_map(|pair| [pair[0], pair[1]])
                .collect(),
            points: path.to_vec(),
            open_ends: (true, true),
        };
        edge_medial::varying_width(&line, wall.role, &wall.flow)
            .into_iter()
            .map(|piece| Run {
                line: match crate::fit(s, &piece.points, wall.role) {
                    Some(fitted) => fitted,
                    None => arc::Fitted::straight(piece.points),
                },
                flow: piece.flow,
            })
            .collect()
    }

    /// Lays a wall with a scarf seam if it is to have one, and says
    /// whether it did. `own` is the island's walls; `path` is the loop
    /// opened at its seam and turned the way it is laid, `at` where on
    /// `wall` it was opened (see [`seam::open_at`]), and `unsupported` how
    /// far the outer wall's line is short of the layer beneath at the
    /// seam the wall starts at or beside.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn scarf_wall(
        &mut self,
        layer: &Layer<'_>,
        own: &[ExtrusionLoop],
        wall: &ExtrusionLoop,
        turned: bool,
        path: &[Point2],
        at: (usize, Point2),
        unsupported: Option<f32>,
    ) -> io::Result<bool> {
        let s = self.settings;
        let scarf = s.scarf;
        if path.len() < 3 {
            return Ok(false);
        }
        // "the walls of a circle under the auto circle compensation
        // print ... with a scarf seam, unless the loop overhangs":
        // whatever else is asked, but "like every scarf here, not on
        // the first layer".
        let circle = scarf.on_circles && !self.state.first_layer && self.circle_wall_whole(wall);
        let asked = self.scarf_asked(wall, path[0], path[1]);
        if !circle && !asked {
            return Ok(false);
        }
        let runs = self.scarf_runs(wall, turned, path, at);
        let round: Vec<Point2> = runs
            .iter()
            .enumerate()
            .flat_map(|(index, run)| run.line.points.iter().skip(usize::from(index > 0)))
            .copied()
            .collect();
        if round.len() < 3 {
            return Ok(false);
        }
        let loop_mm = length(&round) / UNITS_PER_MM;
        let slope_mm = if scarf.entire_loop {
            loop_mm
        } else {
            scarf.min_length.min(loop_mm)
        };
        // A scarf of no length is a plain seam (see the top of this file).
        if slope_mm <= 0.0 {
            return Ok(false);
        }
        if !circle && scarf.conditional {
            let smooth = is_smooth(
                &round[..round.len() - 1],
                scarf.angle_threshold.to_radians(),
                s.nozzle_diameter,
            );
            // The reference asks this of the seam of the outer wall
            // nearest the loop: the wall's own, or the one an inner
            // wall starts beside.
            if !smooth || !self.seam_rests(unsupported, wall.flow.width) {
                return Ok(false);
            }
        }
        // "Get the ratio against current layer height"
        let start_ratio = match scarf.start_height {
            FloatOrPercent::Percent(percent) => percent / 100.0,
            FloatOrPercent::Value(mm) => mm / layer.height,
        };
        let start_ratio = if start_ratio >= 1.0 {
            0.99
        } else {
            start_ratio
        };
        let made = sloped(
            runs,
            arc::tolerance(s.seam_gap),
            slope_mm,
            arc::tolerance(slope_mm / scarf.steps as f64),
            start_ratio,
            wall.role == Role::Perimeter,
        );
        let first = (made.starts.first().map(|start| start.points[0]))
            .or_else(|| made.flat.first().map(|run| run.line.points[0]))
            .or_else(|| made.ends.first().map(|end| end.points[0]));
        let Some(first) = first else {
            return Ok(false);
        };
        // The wall's seam is where it is come at from inside
        // (`wipe.rs`); an outer wall's slope starts two seam gaps on.
        self.wipe_before_wall(own, wall, turned, &round, wall.flow.width)?;
        self.travel(first, false)?;
        for stretch in &made.starts {
            self.lay_stretch(stretch, layer.height, wall.role)?;
        }
        for run in &made.flat {
            // Arcs only where they were asked about.
            let parts = s.arc_fitting.then_some(run.line.parts.as_slice());
            self.lay_path(&run.line.points, parts, wall.role, &run.flow, true)?;
        }
        for stretch in &made.ends {
            self.lay_stretch(stretch, layer.height, wall.role)?;
        }
        // The nozzle is wiped on round the loop from where it stopped:
        // along the flat part and then the way down again ("Fix path
        // for wipe ... The start slope part is ignored as it overlaps
        // with the end part"), or with no way down, along the loop.
        self.state.wipe_path = if made.ends.is_empty() {
            round
        } else {
            (made.flat.iter().map(|run| &run.line.points))
                .chain(made.ends.iter().map(|end| &end.points))
                .flatten()
                .copied()
                .collect()
        };
        Ok(true)
    }

    /// One stretch of a scarf, from where the nozzle is.
    fn lay_stretch(&mut self, stretch: &Stretch, height: f64, role: Role) -> io::Result<()> {
        // A move of next to no length is left out, as the reference
        // leaves it out (GCode.cpp `_extrude`, `line_length < EPSILON`);
        // fitting arcs to a loop and cutting it at its seam can leave one.
        let mut points: Vec<Point2> = Vec::with_capacity(stretch.points.len());
        for &point in &stretch.points {
            match points.last() {
                Some(&last) if span(last, point) < arc::tolerance(EPSILON) => {}
                _ => points.push(point),
            }
        }
        if points.len() < 2 {
            return Ok(());
        }
        self.state.slope = Some(Slope {
            from: stretch.from,
            to: stretch.to,
            total: length(&points) / UNITS_PER_MM,
            done: 0.0,
            height,
        });
        let laid = self.lay_path(&points, None, role, &stretch.flow, true);
        self.state.slope = None;
        laid
    }
}
