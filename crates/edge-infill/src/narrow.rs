//! Narrow solid areas (`detect_narrow_internal_solid_infill`): the parts
//! of the solid fill inside a part that straight lines would cross in
//! strokes too short to be worth laying, told from the rest and filled
//! with loops along their outline instead, of whatever width fits.
//!
//! A port of the reference's `split_solid_surface` and the
//! `filter_vibrating_extrusions` it calls (Fill/Fill.cpp, itself after
//! PrusaSlicer's FillEnsuring.cpp), of where `group_fills` uses them, and
//! of the fill it gives the narrow part (Fill/FillConcentricInternal.cpp).

use std::collections::VecDeque;
use std::f64::consts::FRAC_PI_2;

use edge_clip::{ClipError, Join};
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_geom::{Coord, ExPolygon, Point2, Polygon, scaled};
use edge_medial::ThickPolyline;

/// What filling narrow solid areas with loops needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NarrowFill {
    /// The smallest nozzle on the printer, in mm: the loops' least width
    /// and the thinnest part that gets one are shares of it.
    pub nozzle: f64,
    /// How far short of where it began a loop stops, in mm (`seam_gap`).
    pub seam_gap: f64,
}

/// A solid area told apart: what is filled with lines, and what with loops.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Split {
    pub wide: Vec<ExPolygon>,
    pub narrow: Vec<ExPolygon>,
}

/// A line shorter than this is one a part would be shaken by, were
/// there many in a row (`_MAX_LINE_LENGTH_TO_FILTER`).
const SHORT: f64 = 4.0;
/// How many long lines a run of short ones may pass through and still
/// be one run (`MAX_SKIPS_ALLOWED`).
const SKIPS: i32 = 2;
/// A short line this deep in a run of them goes
/// (`MIN_DEPTH_FOR_LINE_REMOVING`).
const DEPTH: i32 = 5;
/// `ClipperSafetyOffset` (ClipperUtils.hpp).
const SAFETY_OFFSET: Coord = 10;

/// One stroke of a fill line across the area, in the frame where lines
/// run up and down: from `low` to `high` at its column's `x`.
#[derive(Clone, Debug, Default)]
struct Stroke {
    low: Coord,
    high: Coord,
    /// The strokes of the columns before and after that it lies beside.
    before: Vec<usize>,
    after: Vec<usize>,
    removed: bool,
    state: State,
}

/// What is known of the run of short strokes a stroke was reached by.
#[derive(Clone, Copy, Debug, Default)]
struct State {
    /// The fewest long strokes passed on the way here.
    skips: i32,
    /// The short strokes passed on the way here.
    short: i32,
    /// Whether the run began beside a long stroke.
    began_beside_long: bool,
    set: bool,
}

impl Stroke {
    fn is_short(&self) -> bool {
        ((self.high - self.low) as f64) < scaled(SHORT) as f64
    }
}

/// Whether a stroke is to go: a short one deep in a run of short ones,
/// or one with nothing kept after it, in a run that did not begin
/// beside a long stroke (`can_line_note_be_removed`).
///
/// Where its run began is what the stroke itself was last told: by the
/// run that reached it, or of itself where a run began with it. So a
/// short stroke that ends the lines right after a long one stays a
/// line, and of short strokes running out into a corner the first
/// stays and loops are made of the rest.
///
/// Not as the reference, where this goes against what it is plainly
/// for. Its `is_initial_line_touching_long_lines` is described as
/// answering for the stroke asked about ("this information was
/// propagated into the current line") and reads what the strokes
/// before it were told instead, in this pass over the columns or an
/// earlier one. The stroke a run begins with is then never asked what
/// was noted of it, only the strokes after it are: a lone short stroke
/// after a long one goes, the very one the note is made to keep, and is
/// laid by itself as a narrow area; and of a run into a corner the
/// first two stay, the second on what the first was told a pass
/// before. What `filter_vibrating_extrusions` was written to replace
/// (PrusaSlicer's FillEnsuring.cpp before 2.9) asked every stroke of a
/// run, the first too, where that run began.
fn goes(columns: &[Vec<Stroke>], column: usize, index: usize) -> bool {
    let stroke = &columns[column][index];
    if !stroke.is_short() {
        return false;
    }
    if stroke.state.short > DEPTH {
        return true;
    }
    let carries_on = stroke
        .after
        .iter()
        .any(|&other| !columns[column + 1][other].removed);
    !stroke.state.began_beside_long && !carries_on
}

/// Takes out of the strokes of an area, column by column, the short ones
/// that would be laid one after another: what is left is where lines are
/// worth laying (`filter_vibrating_extrusions`).
fn filter(columns: &mut [Vec<Stroke>]) {
    let beside = |a: &Stroke, b: &Stroke| a.low <= b.high && b.low <= a.high;
    for column in 1..columns.len() {
        let (head, tail) = columns.split_at_mut(column);
        let (earlier, later) = (&mut head[column - 1], &mut tail[0]);
        for (at, stroke) in later.iter_mut().enumerate() {
            for (other_at, other) in earlier.iter_mut().enumerate() {
                if beside(stroke, other) {
                    stroke.before.push(other_at);
                    other.after.push(at);
                }
            }
        }
    }

    for start in 0..columns.len() {
        // A run starts from each short stroke of this column.
        for index in 0..columns[start].len() {
            let stroke = &columns[start][index];
            if stroke.removed || !stroke.is_short() {
                continue;
            }
            let began_beside_long = stroke.before.iter().any(|&other| {
                let other = &columns[start - 1][other];
                !other.removed && !other.is_short()
            });
            columns[start][index].state = State {
                skips: 0,
                short: 1,
                began_beside_long,
                set: true,
            };
        }
        // And is carried on, column by column, to the last.
        for column in start..columns.len() {
            if column + 1 < columns.len() {
                for stroke in &mut columns[column + 1] {
                    stroke.state = State::default();
                }
            }
            for index in 0..columns[column].len() {
                let (state, after) = {
                    let stroke = &columns[column][index];
                    if stroke.removed || !stroke.state.set {
                        continue;
                    }
                    (stroke.state, stroke.after.clone())
                };
                for other in after {
                    let next = &mut columns[column + 1][other];
                    if next.removed {
                        continue;
                    }
                    let is_short = next.is_short();
                    if !is_short && state.skips >= SKIPS {
                        continue;
                    }
                    let short = state.short + i32::from(is_short);
                    let skips = state.skips + i32::from(!is_short);
                    if next.state.set {
                        // Reached a second way: kept is whatever makes
                        // it likelier to go. The reference means this
                        // too ("update data in such a way that will
                        // maximize the possibility of removing this
                        // node") but writes the greater count of short
                        // strokes into the count of skips, and the
                        // count of short strokes stays the first
                        // comer's. That mistake is not carried over.
                        next.state.short = next.state.short.max(short);
                        next.state.skips = next.state.skips.min(skips);
                        if next.state.began_beside_long {
                            next.state.began_beside_long = state.began_beside_long;
                        }
                    } else {
                        next.state = State {
                            skips,
                            short,
                            began_beside_long: state.began_beside_long,
                            set: true,
                        };
                    }
                }
                if goes(columns, column, index) {
                    columns[column][index].removed = true;
                    // And so, back along the run, does every stroke
                    // this leaves with no reason to stay
                    // (`propagate_line_node_remove`).
                    let mut queue: VecDeque<(usize, usize)> = columns[column][index]
                        .before
                        .iter()
                        .filter(|&&other| !columns[column - 1][other].removed)
                        .map(|&other| (column - 1, other))
                        .collect();
                    while let Some((column, index)) = queue.pop_front() {
                        if goes(columns, column, index) {
                            columns[column][index].removed = true;
                            for &other in &columns[column][index].before {
                                if !columns[column - 1][other].removed {
                                    queue.push_back((column - 1, other));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The outline of what a part's kept strokes cover, each stroke as wide
/// as the lines are apart and half that longer at either end: polygons
/// drawn column by column, the bottom ends of a run of strokes one way
/// and the top ends back ("reconstruct polygon from polygon sections").
fn outline(columns: &[Vec<(Coord, Coord, Coord)>], spacing: Coord) -> Vec<Polygon> {
    struct Traced {
        lows: Vec<Point2>,
        highs: Vec<Point2>,
    }
    let half = spacing / 2;
    let step = |point: Point2, by: Coord| Point2 {
        x: point.x + by,
        y: point.y,
    };
    // Two strokes this near are joined corner to corner.
    let near = 2.0 * spacing as f64;
    let is_near = |a: Point2, b: Point2| ((a.x - b.x) as f64).hypot((a.y - b.y) as f64) < near;
    let close = |traced: Traced| Polygon {
        points: traced
            .lows
            .into_iter()
            .chain(traced.highs.into_iter().rev())
            .collect(),
    };
    let mut done = Vec::new();
    let mut open: Vec<Traced> = Vec::new();
    for strokes in columns {
        let mut used = vec![false; strokes.len()];
        let mut carried = Vec::with_capacity(open.len());
        for mut traced in open {
            let (low, high) = (
                traced.lows[traced.lows.len() - 1],
                traced.highs[traced.highs.len() - 1],
            );
            // The strokes of this column beside the last one traced.
            let from = strokes.partition_point(|stroke| stroke.2 <= low.y);
            let to = strokes.partition_point(|stroke| stroke.1 <= high.y);
            // The first of them not taken already carries the polygon
            // on. The reference looks for it the same way and then
            // takes the first of them whether or not it is free
            // (`candidates_begin` where `candidate` is meant), so two
            // runs that come to the same pair of strokes both go on
            // into the first and the second starts a polygon of its
            // own. That is not carried over.
            match (from..to).find(|&index| !used[index]) {
                Some(index) => {
                    used[index] = true;
                    let (x, bottom, top) = strokes[index];
                    for (ends, to) in [
                        (&mut traced.lows, Point2 { x, y: bottom }),
                        (&mut traced.highs, Point2 { x, y: top }),
                    ] {
                        let last = ends[ends.len() - 1];
                        if !is_near(last, to) {
                            ends.push(step(last, half));
                            ends.push(step(to, -half));
                        }
                        ends.push(to);
                    }
                    carried.push(traced);
                }
                None => {
                    traced.lows.push(step(low, half));
                    traced.highs.push(step(high, half));
                    done.push(close(traced));
                }
            }
        }
        open = carried;
        for (index, &(x, bottom, top)) in strokes.iter().enumerate() {
            if !used[index] {
                let (low, high) = (Point2 { x, y: bottom }, Point2 { x, y: top });
                open.push(Traced {
                    lows: vec![step(low, -half), low],
                    highs: vec![step(high, -half), high],
                });
            }
        }
    }
    // The reference closes what reaches the last column where its
    // strokes are, without the half step every other end gets.
    done.extend(open.into_iter().map(close));
    done
}

/// Tells the narrow parts of solid areas from the rest, for lines laid
/// `spacing` apart in `direction`, radians counter-clockwise from +x
/// (`split_solid_surface`, as it is done for the patterns of straight
/// lines, which are all this engine lays there). None when nothing is
/// narrow, and the areas are then filled as they are.
///
/// Where lines would be laid is worked out much as the fill lays them:
/// the area less what is under four spacings wide, half a spacing in
/// from its edge, crossed by a line every spacing; a stroke no longer
/// than a spacing is none. Of those strokes the short ones that would
/// follow one another are taken out ([`filter`]), and what the rest
/// cover is the wide part.
pub(crate) fn split(
    areas: &[ExPolygon],
    spacing: Coord,
    direction: f64,
) -> Result<Option<Split>, ClipError> {
    if areas.is_empty() || spacing <= 0 {
        return Ok(None);
    }
    // Turned as the reference turns the area (`aligning_angle`), so the
    // columns are counted from the same side.
    let (sin, cos) = (FRAC_PI_2 - direction).sin_cos();
    let turned = |p: Point2| Point2 {
        x: (p.x as f64 * cos - p.y as f64 * sin).round() as Coord,
        y: (p.x as f64 * sin + p.y as f64 * cos).round() as Coord,
    };
    let back = |p: Point2| Point2 {
        x: (p.x as f64 * cos + p.y as f64 * sin).round() as Coord,
        y: (p.y as f64 * cos - p.x as f64 * sin).round() as Coord,
    };
    let mut covered: Vec<Polygon> = Vec::new();
    for area in areas {
        let one = std::slice::from_ref(area);
        let core = edge_clip::intersection(
            one,
            &edge_clip::offset2(one, -2 * spacing, 3 * spacing, Join::MITER)?,
        )?;
        let core = edge_clip::offset(&core, -(spacing / 2), Join::MITER)?;
        let Some((left, right)) = area
            .contour
            .points
            .iter()
            .map(|&point| turned(point).x)
            .fold(None, |ends: Option<(Coord, Coord)>, x| match ends {
                None => Some((x, x)),
                Some((left, right)) => Some((left.min(x), right.max(x))),
            })
        else {
            continue;
        };
        let edges: Vec<(Point2, Point2)> = core
            .iter()
            .flat_map(ExPolygon::rings)
            .flat_map(|ring| {
                let points = &ring.points;
                (0..points.len())
                    .map(move |at| (points[at], points[(at + 1) % points.len()]))
                    .map(|(a, b)| (turned(a), turned(b)))
            })
            .filter(|(a, b)| a.x != b.x)
            .collect();
        let count = ((right - left + spacing - 1) / spacing) as usize;
        let mut columns: Vec<Vec<Stroke>> = Vec::with_capacity(count);
        let mut places = Vec::with_capacity(count);
        for column in 0..count {
            let x = left + column as Coord * spacing;
            // Where the column crosses the outline: in at one crossing
            // and out at the next.
            let mut crossings: Vec<f64> = edges
                .iter()
                .filter(|(a, b)| a.x.min(b.x) <= x && x < a.x.max(b.x))
                .map(|(a, b)| {
                    a.y as f64 + (b.y - a.y) as f64 * (x - a.x) as f64 / (b.x - a.x) as f64
                })
                .collect();
            crossings.sort_by(f64::total_cmp);
            columns.push(
                crossings
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|pair| (pair[0].round() as Coord, pair[1].round() as Coord))
                    .filter(|(low, high)| high - low > spacing)
                    .map(|(low, high)| Stroke {
                        low,
                        high,
                        ..Stroke::default()
                    })
                    .collect(),
            );
            places.push(x);
        }
        filter(&mut columns);
        let kept: Vec<Vec<(Coord, Coord, Coord)>> = columns
            .iter()
            .zip(&places)
            .map(|(strokes, &x)| {
                strokes
                    .iter()
                    .filter(|stroke| !stroke.removed)
                    .map(|stroke| (x, stroke.low - spacing / 2, stroke.high + spacing / 2))
                    .collect()
            })
            .collect();
        covered.extend(outline(&kept, spacing).into_iter().map(|polygon| Polygon {
            points: polygon.points.into_iter().map(back).collect(),
        }));
    }

    let mut wide = edge_clip::offset(&edge_clip::union(&covered)?, SAFETY_OFFSET, Join::MITER)?;
    let mut narrow = Vec::new();
    for piece in edge_clip::difference(areas, &wide)? {
        // A piece too small for a line of its own goes with the wide
        // part where it touches it: the strokes stop half a spacing
        // short of a slanting edge, and leave such pieces all along it.
        let one = std::slice::from_ref(&piece);
        if edge_clip::offset(one, -(spacing / 2), Join::MITER)?.is_empty() {
            let around = edge_clip::offset(one, (0.3 * spacing as f64) as Coord, Join::MITER)?;
            if !edge_clip::intersection(&wide, &around)?.is_empty() {
                wide.push(piece);
                continue;
            }
        }
        narrow.push(piece);
    }
    if narrow.is_empty() {
        return Ok(None);
    }
    // The wide part was drawn half a spacing in; out again by as much,
    // so nothing is left bare between the two.
    let wide =
        edge_clip::intersection(&edge_clip::offset(&wide, spacing / 2, Join::MITER)?, areas)?;
    Ok(Some(Split { wide, narrow }))
}

/// Stops a line `distance` short of its end (`Polyline::clip_end`); it
/// is left with fewer than two points when it is no longer than that.
fn clip_end(line: &mut ThickPolyline, mut distance: f64) {
    while distance > 0.0 {
        let Some(last) = line.points.pop() else {
            break;
        };
        let Some(&before) = line.points.last() else {
            break;
        };
        let (dx, dy) = ((before.x - last.x) as f64, (before.y - last.y) as f64);
        let length = dx.hypot(dy);
        if length > distance {
            let share = distance / length;
            line.points.push(Point2 {
                x: last.x + (dx * share) as Coord,
                y: last.y + (dy * share) as Coord,
            });
            break;
        }
        distance -= length;
    }
    line.widths
        .truncate(2 * line.points.len().saturating_sub(1));
}

/// The loops that fill a narrow area, in the order they are to be laid
/// and each from its first point (`FillConcentricInternal`): walls of
/// changing width, as many as fit, drawn inside the part of the area
/// that lies within `inside`, the layer's fill area without its overlap
/// into the walls; with none given, inside the area itself. Each loop
/// stops the seam gap short of where it began, and is cut into paths
/// where its width changes.
///
/// Not as the reference, each where it goes against what it is plainly
/// for: there every loop starts at its point nearest the object's own
/// origin, the place the last one ended at never being noted (`last_pos`
/// is set once and not again, where FillConcentric.cpp sets it after
/// every line); here each starts nearest where the one before it ended,
/// which for a loop is where it began.
/// And there a line that is no loop loses the seam gap from its end as
/// well; here only loops do.
pub(crate) fn loops(
    area: &ExPolygon,
    inside: &[ExPolygon],
    flow: &Flow,
    narrow: &NarrowFill,
    part: u32,
) -> Result<Vec<ExtrusionPath>, crate::FillError> {
    let pieces = if inside.is_empty() {
        vec![area.clone()]
    } else {
        edge_clip::intersection(
            inside,
            &edge_clip::offset(std::slice::from_ref(area), SAFETY_OFFSET, Join::MITER)?,
        )?
    };
    let spacing = flow.scaled_spacing();
    let nozzle = narrow.nozzle;
    let mut lines: Vec<ThickPolyline> = Vec::new();
    for piece in &pieces {
        let Some((low, high)) = bounds(&piece.contour) else {
            continue;
        };
        let loops = ((high.x - low.x).max(high.y - low.y) / spacing + 1) as usize;
        let walls = edge_arachne::generate(
            std::slice::from_ref(piece),
            &edge_arachne::Params {
                bead_width_0: spacing,
                bead_width_x: spacing,
                inset_count: loops,
                wall_0_inset: 0,
                layer_height: flow.height,
                min_bead_width: (0.85 * nozzle) as f32,
                min_feature_size: (0.25 * nozzle) as f32,
                // The reference's own, whatever the walls are set to
                // (`WallToolPathsParams`'s defaults).
                min_length_factor: 0.5,
                wall_transition_length: 0.4,
                wall_transition_angle: 10.0,
                wall_transition_filter_deviation: (0.25 * nozzle) as f32,
                wall_distribution_count: 1,
                is_top_or_bottom_layer: false,
                wall_maximum_resolution: scaled(0.5),
                wall_maximum_deviation: scaled(0.025),
            },
        )?;
        let first = lines.len();
        let mut last_end = Point2 { x: 0, y: 0 };
        for wall in walls.toolpaths.iter().flatten() {
            if wall.junctions.len() < 2 {
                continue;
            }
            let mut line = ThickPolyline {
                points: wall.junctions.iter().map(|junction| junction.p).collect(),
                widths: wall
                    .junctions
                    .windows(2)
                    .flat_map(|pair| [pair[0].w as f64, pair[1].w as f64])
                    .collect(),
                open_ends: (true, true),
            };
            let count = line.points.len();
            let closed = wall.is_closed
                && line.points[0] == line.points[count - 1]
                && line.widths[0] == line.widths[line.widths.len() - 1];
            if closed {
                // From the point nearest where the last line ended,
                // the first being as near the origin as the reference
                // has every one.
                line.points.pop();
                let nearest = (0..line.points.len())
                    .min_by_key(|&at| {
                        let (dx, dy) = (
                            i128::from(line.points[at].x - last_end.x),
                            i128::from(line.points[at].y - last_end.y),
                        );
                        dx * dx + dy * dy
                    })
                    .unwrap_or(0);
                line.points.rotate_left(nearest);
                line.widths.rotate_left(2 * nearest);
                line.points.push(line.points[0]);
            }
            last_end = line.points[line.points.len() - 1];
            if closed {
                clip_end(&mut line, scaled(narrow.seam_gap) as f64);
            }
            if line.points.len() >= 2 {
                lines.push(line);
            }
        }
        // Each after the one that starts nearest it. The reference
        // orders them by its chain of nearest pairs
        // (`reorder_by_shortest_traverse`); here from the first, the
        // nearest start next.
        nearest_next(&mut lines[first..]);
    }
    let mut paths = Vec::new();
    for line in &lines {
        paths.extend(edge_medial::varying_width(line, Role::SolidInfill, flow));
    }
    for (index, path) in paths.iter_mut().enumerate() {
        path.part = part;
        path.in_turn = index > 0;
    }
    Ok(paths)
}

/// Puts lines in the order of a walk from the first to whichever starts
/// nearest the start of the one before.
fn nearest_next(lines: &mut [ThickPolyline]) {
    for done in 1..lines.len() {
        let from = lines[done - 1].points[0];
        let nearest = (done..lines.len())
            .min_by_key(|&at| {
                let to = lines[at].points[0];
                let (dx, dy) = (i128::from(to.x - from.x), i128::from(to.y - from.y));
                dx * dx + dy * dy
            })
            .unwrap_or(done);
        lines.swap(done, nearest);
    }
}

/// The least and greatest corners of the box round a ring.
fn bounds(ring: &Polygon) -> Option<(Point2, Point2)> {
    let first = *ring.points.first()?;
    Some(ring.points.iter().fold((first, first), |(low, high), p| {
        (
            Point2 {
                x: low.x.min(p.x),
                y: low.y.min(p.y),
            },
            Point2 {
                x: high.x.max(p.x),
                y: high.y.max(p.y),
            },
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    fn area(regions: &[ExPolygon]) -> f64 {
        regions.iter().map(ExPolygon::area_mm2).sum()
    }

    /// Lines 0.4 mm apart.
    const SPACING: Coord = 400_000;

    #[test]
    fn a_wide_area_square_to_its_lines_is_all_lines() {
        // Lines along x and along y: no stroke is short, and the half
        // spacing the strokes stop short at either side goes with them.
        let square = [rect(0.0, 0.0, 20.0, 20.0)];
        assert_eq!(split(&square, SPACING, 0.0).unwrap(), None);
        assert_eq!(split(&square, SPACING, FRAC_PI_2).unwrap(), None);
        assert_eq!(split(&[], SPACING, 0.0).unwrap(), None);
    }

    #[test]
    fn a_strip_under_four_lines_wide_is_all_loops() {
        // 1.2 mm is three lines: the reference takes anything under
        // four for narrow, whichever way the lines run.
        let strip = [rect(0.0, 0.0, 20.0, 1.2)];
        for direction in [0.0, FRAC_PI_2, 0.7] {
            let told = split(&strip, SPACING, direction).unwrap().unwrap();
            assert!(told.wide.is_empty(), "{direction}");
            assert!((area(&told.narrow) - 24.0).abs() < 1e-6, "{direction}");
        }
        // Two millimetres is five: wide, with lines along it.
        assert_eq!(
            split(&[rect(0.0, 0.0, 20.0, 2.0)], SPACING, 0.0).unwrap(),
            None
        );
    }

    #[test]
    fn a_thin_arm_on_a_wide_area_is_told_from_it() {
        // A 10 mm square with an arm 1 mm wide and 10 mm long out of
        // the middle of its right side.
        let shape = [ExPolygon {
            contour: Polygon {
                points: [
                    (0.0, 0.0),
                    (10.0, 0.0),
                    (10.0, 4.5),
                    (20.0, 4.5),
                    (20.0, 5.5),
                    (10.0, 5.5),
                    (10.0, 10.0),
                    (0.0, 10.0),
                ]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            },
            holes: Vec::new(),
        }];
        for direction in [0.0, FRAC_PI_2] {
            let told = split(&shape, SPACING, direction).unwrap().unwrap();
            // The arm, from where the square's lines stop to its end.
            // With lines up and down, the last of the square's stops
            // short of its side, and what it leaves there, thinner
            // than a line and joined to the arm, is narrow with it:
            // so the reference has it, and the lines are then brought
            // back out over it.
            assert_eq!(told.narrow.len(), 1, "{direction}");
            let arm = area(&told.narrow);
            let most = if direction == 0.0 { 10.5 } else { 12.1 };
            assert!((9.5..=most).contains(&arm), "{direction}: {arm}");
            let within = |point: &Point2| {
                point.x >= 9_500_000
                    && (direction != 0.0 || (4_499_000..=5_501_000).contains(&point.y))
            };
            assert!(
                told.narrow[0].contour.points.iter().all(within),
                "{direction}"
            );
            // The square, and half a spacing into the arm so nothing is
            // bare between the two.
            let wide = area(&told.wide);
            assert!((100.0..=101.0).contains(&wide), "{direction}: {wide}");
            let both = edge_clip::intersection(&told.wide, &told.narrow).unwrap();
            assert!(area(&both) < most - 9.5, "{direction}: {}", area(&both));
            let all = edge_clip::union_ex(
                &told
                    .wide
                    .iter()
                    .chain(&told.narrow)
                    .cloned()
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            assert!((area(&all) - 110.0).abs() < 1e-3, "{direction}");
        }
    }

    #[test]
    fn slanting_lines_leave_a_corner_of_a_square_to_loops() {
        // Lines at 45 degrees get shorter into two corners of a square.
        // Where the strokes run out, the first short one stays beside
        // the last long one and the rest, with nothing kept after
        // them, go; where they begin, the short ones lead into long
        // ones and stay. So the corner the strokes end in is narrow: a
        // small triangle of loops in one corner of every solid layer
        // of a cube. The reference has it one stroke smaller (1.65 mm2
        // of this square), keeping the first two short strokes. The
        // corner they begin in keeps all but its very tip, which no
        // stroke reaches and is too small for anything.
        let square = [rect(0.0, 0.0, 20.0, 20.0)];
        let told = split(&square, SPACING, 45f64.to_radians())
            .unwrap()
            .unwrap();
        let middle = |area: &ExPolygon| {
            let points = &area.contour.points;
            let count = points.len() as f64;
            (
                points.iter().map(|p| p.x as f64).sum::<f64>() / count / 1e6,
                points.iter().map(|p| p.y as f64).sum::<f64>() / count / 1e6,
            )
        };
        let mut corners: Vec<(f64, (f64, f64))> = told
            .narrow
            .iter()
            .map(|area| (area.area_mm2(), middle(area)))
            .collect();
        corners.sort_by(|a, b| a.0.total_cmp(&b.0));
        let [(tip, (tip_x, tip_y)), (corner, (x, y))] = corners[..] else {
            panic!("{corners:?}")
        };
        // The corner the columns end in, as the reference counts them.
        assert!((2.6..3.1).contains(&corner), "{corner}");
        assert!(x > 18.0 && y < 2.0, "{x} {y}");
        assert!(
            tip < 0.5 && tip_x < 1.0 && tip_y > 19.0,
            "{tip} {tip_x} {tip_y}"
        );
        assert!((area(&told.wide) - 400.0).abs() < 3.0);
    }

    fn strokes(spans: &[&[(f64, f64)]]) -> Vec<Vec<Stroke>> {
        spans
            .iter()
            .map(|column| {
                column
                    .iter()
                    .map(|&(low, high)| Stroke {
                        low: scaled(low),
                        high: scaled(high),
                        ..Stroke::default()
                    })
                    .collect()
            })
            .collect()
    }

    fn removed(columns: &[Vec<Stroke>]) -> Vec<Vec<bool>> {
        columns
            .iter()
            .map(|column| column.iter().map(|stroke| stroke.removed).collect())
            .collect()
    }

    #[test]
    fn short_strokes_go_where_they_follow_one_another() {
        // Long strokes stay.
        let mut long = strokes(&[&[(0.0, 10.0)], &[(0.0, 10.0)], &[(0.0, 10.0)]]);
        filter(&mut long);
        assert_eq!(removed(&long), [[false], [false], [false]]);
        // A run of short ones that begins and ends at nothing goes.
        let mut short = strokes(&[&[(0.0, 2.0)], &[(0.0, 2.0)], &[(0.0, 2.0)], &[(0.0, 2.0)]]);
        filter(&mut short);
        assert_eq!(removed(&short), [[true], [true], [true], [true]]);
        // Three short strokes between long ones stay: the run is not
        // deep, and it goes on into a long stroke.
        let mut between = strokes(&[
            &[(0.0, 10.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 10.0)],
        ]);
        filter(&mut between);
        assert_eq!(removed(&between), [[false]; 5]);
        // Eight do not: from the sixth of the run they go, and so back
        // along the run does every one left with nothing after it.
        let mut deep = strokes(&[
            &[(0.0, 10.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 2.0)],
            &[(0.0, 10.0)],
        ]);
        filter(&mut deep);
        let gone = removed(&deep);
        assert_eq!((gone[0][0], gone[9][0]), (false, false));
        assert!(gone[6..9].iter().all(|column| column[0]), "{gone:?}");
    }

    #[test]
    fn where_a_run_began_is_asked_of_the_stroke_itself() {
        // Lines running out into a corner: after the last long stroke,
        // five short ones and then nothing. The run from the first of
        // them began beside a long stroke and all five stay for it;
        // the run from the second did not, and goes, back to the
        // stroke it began with. The first stays. (In the reference
        // the second stays too: it is asked what the first was told,
        // not what it was.)
        let mut corner = strokes(&[
            &[(0.0, 10.0)],
            &[(0.0, 3.5)],
            &[(0.0, 2.8)],
            &[(0.0, 2.0)],
            &[(0.0, 1.3)],
            &[(0.0, 0.5)],
        ]);
        filter(&mut corner);
        assert_eq!(
            removed(&corner),
            [[false], [false], [true], [true], [true], [true]]
        );
        // One short stroke after a long one and nothing after it: a
        // run that began beside a long stroke, and it stays. (The
        // reference asks the long stroke, which no run has reached,
        // and takes the short one out.)
        let mut lone = strokes(&[&[(0.0, 10.0)], &[(0.0, 10.0)], &[(0.0, 2.4)]]);
        filter(&mut lone);
        assert_eq!(removed(&lone), [[false], [false], [false]]);
        // With no long stroke before it, it goes.
        let mut alone = strokes(&[&[(20.0, 30.0)], &[(0.0, 2.4)]]);
        filter(&mut alone);
        assert_eq!(removed(&alone), [[false], [true]]);
        // Short strokes that lead into long ones stay, wherever they
        // began: the corner the lines begin in.
        let mut leading = strokes(&[
            &[(0.0, 1.0)],
            &[(0.0, 1.7)],
            &[(0.0, 2.5)],
            &[(0.0, 3.2)],
            &[(0.0, 10.0)],
        ]);
        filter(&mut leading);
        assert_eq!(removed(&leading), [[false]; 5]);
    }

    #[test]
    fn a_stroke_reached_two_ways_counts_the_deeper_run() {
        // Two runs from the first column meet in a short stroke of the
        // sixth: the lower one passes two long strokes and counts three
        // short ones, the upper counts five. The stroke they meet in is
        // the sixth of the upper run and goes, though the lower run
        // reaches it first. (The reference keeps the first comer's
        // count, by a slip of the pen, and the stroke with it.)
        let mut columns = strokes(&[
            &[(0.0, 2.0), (6.0, 8.0)],
            &[(0.0, 4.5), (6.0, 8.0)],
            &[(0.0, 4.5), (6.0, 8.0)],
            &[(0.0, 2.0), (6.0, 8.0)],
            &[(0.0, 2.0), (4.0, 7.0)],
            &[(1.5, 4.5)],
            &[(0.0, 10.0)],
        ]);
        filter(&mut columns);
        assert!(columns[5][0].removed, "{:?}", removed(&columns));
        assert!(!columns[6][0].removed);
    }

    #[test]
    fn a_loop_stops_short_of_where_it_began() {
        let mut line = ThickPolyline {
            points: [(0.0, 0.0), (10.0, 0.0), (10.0, 1.0), (0.0, 1.0), (0.0, 0.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
            widths: vec![400_000.0; 8],
            open_ends: (true, true),
        };
        clip_end(&mut line, scaled(0.25) as f64);
        assert_eq!(line.points.len(), 5);
        assert_eq!(line.points[4], Point2::from_mm(0.0, 0.25));
        assert_eq!(line.widths.len(), 8);
        // More than the last stretch: that goes, and the rest comes
        // off the one before.
        clip_end(&mut line, scaled(1.75) as f64);
        assert_eq!(line.points.len(), 4);
        assert_eq!(line.points[3], Point2::from_mm(1.0, 1.0));
        assert_eq!(line.widths.len(), 6);
        // More than there is leaves no line.
        clip_end(&mut line, scaled(50.0) as f64);
        assert!(line.points.len() < 2);
    }

    #[test]
    fn a_narrow_strip_is_filled_with_loops_of_its_own_width() {
        let flow = Flow::new(0.45, 0.2, 0.4).unwrap();
        let narrow = NarrowFill {
            nozzle: 0.4,
            seam_gap: 0.06,
        };
        let strip = rect(0.0, 0.0, 20.0, 1.2);
        let paths = loops(&strip, &[], &flow, &narrow, 7).unwrap();
        assert!(!paths.is_empty());
        // Solid fill, one part, laid in the order given.
        assert!(paths.iter().all(|path| path.role == Role::SolidInfill));
        assert!(paths.iter().all(|path| path.part == 7));
        assert!(!paths[0].in_turn && paths[1..].iter().all(|path| path.in_turn));
        // Inside the strip, and as much plastic as fills it.
        assert!(paths.iter().flat_map(|path| &path.points).all(|point| {
            (0..=20_000_000).contains(&point.x) && (0..=1_200_000).contains(&point.y)
        }));
        let volume: f64 = paths.iter().map(ExtrusionPath::volume_mm3).sum();
        let whole = 20.0 * 1.2 * 0.2;
        assert!((volume / whole - 1.0).abs() < 0.08, "{volume} of {whole}");
        // Three lines' worth across: a loop, which is two, and one down
        // the middle.
        let length: f64 = paths.iter().map(ExtrusionPath::length_mm).sum();
        assert!((55.0..=62.0).contains(&length), "{length}");
        // The loop does not come back to where it began.
        let (first, last) = (
            paths[0].points[0],
            paths
                .iter()
                .rev()
                .map(|path| path.points[path.points.len() - 1])
                .find(|end| {
                    let (dx, dy) = (
                        (end.x - paths[0].points[0].x) as f64,
                        (end.y - paths[0].points[0].y) as f64,
                    );
                    dx.hypot(dy) < 1e6
                }),
        );
        let last = last.expect("an end near the loop's start");
        let gap = ((last.x - first.x) as f64).hypot((last.y - first.y) as f64) / 1e6;
        assert!((gap - 0.06).abs() < 1e-3, "{gap}");

        // Kept inside what is given as inside the walls.
        let inside = [rect(0.0, 0.2, 20.0, 1.2)];
        let within = loops(&strip, &inside, &flow, &narrow, 7).unwrap();
        assert!(
            within
                .iter()
                .flat_map(|path| &path.points)
                .all(|point| point.y >= 200_000)
        );
    }
}
