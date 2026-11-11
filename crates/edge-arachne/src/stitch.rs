//! Joins the pieces of wall the generator leaves into longer lines, and
//! into closed loops where a line comes back to its own start: from the
//! end of a line it looks for the nearest end of another within reach and
//! carries on along that one.
//!
//! A port of the reference's
//! `src/libslic3r/Arachne/utils/PolylineStitcher.hpp` and
//! `PolylineStitcher.cpp` (from CuraEngine), for wall lines only
//! (`PolylineStitcher<VariableWidthLines, ExtrusionLine, ExtrusionJunction>`),
//! with what it uses of `SparsePointGrid.hpp`, `SparseGrid.hpp`,
//! `SquareGrid.hpp`/`.cpp` and `PolygonsPointIndex.hpp`.
//!
//! Three things the reference does that are easy to take for mistakes
//! here:
//! - only `is_odd` decides whether two lines may be joined; `inset_idx` is
//!   not looked at, because the caller hands over one wall's lines at a
//!   time. A joined line has the `inset_idx` of its first piece.
//! - a line that may be turned round (an odd one) comes out turned round
//!   unless it was closed into a loop.
//! - a loop comes out in `result_polygons` as it was walked: `is_closed`
//!   is left as the first piece had it and the first junction is not
//!   repeated at the end unless the last piece ended exactly on it.
//!   `WallToolPaths::stitchToolPaths` sets both afterwards.

use std::collections::HashMap;

use edge_geom::{Coord, Point2};

use crate::line::{v_size, v_size2};
use crate::{ExtrusionLine, VariableWidthLines};

/// The gap `stitch` bridges when the reference is not told otherwise:
/// `scaled<coord_t>(0.1)`.
pub const DEFAULT_MAX_STITCH_DISTANCE: Coord = 100_000;

/// The distance under which `stitch` takes two points for one when the
/// reference is not told otherwise: `scaled<coord_t>(0.01)`.
pub const DEFAULT_SNAP_DISTANCE: Coord = 10_000;

/// `scaled<coord_t>(0.01)`: how much nearer or further a closing end is
/// made to look, to prefer or put off closing a loop.
const CLOSING_PREFERENCE: Coord = 10_000;

/// One end of one of the lines being stitched (`PathsPointIndex`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LineEnd {
    /// Which line.
    line_idx: usize,
    /// Which junction of it: 0, or the last.
    point_idx: usize,
}

/// The ends of the lines, sorted into square cells so the ones near a
/// point are found without going through all of them (`SparsePointGrid`
/// over `SquareGrid`).
struct EndGrid {
    cell_size: Coord,
    /// The map is only ever asked for one cell by its key, never walked,
    /// so its order does not reach the result.
    cells: HashMap<(i64, i64), Vec<LineEnd>>,
}

impl EndGrid {
    fn new(cell_size: Coord, reserve: usize) -> Self {
        Self {
            cell_size,
            cells: HashMap::with_capacity(reserve),
        }
    }

    /// The cell a coordinate is in. The division cuts toward zero, so the
    /// cells along each axis are twice as wide as the rest
    /// (`SquareGrid::toGridCoord`).
    fn to_grid_coord(&self, coord: Coord) -> i64 {
        coord / self.cell_size
    }

    fn insert(&mut self, at: Point2, end: LineEnd) {
        let cell = (self.to_grid_coord(at.x), self.to_grid_coord(at.y));
        self.cells.entry(cell).or_default().push(end);
    }

    /// Hands `process` every end in the cells a square of `radius` round
    /// `query` touches, until it returns false; returns whether it never
    /// did (`SquareGrid::processNearby` with `SparseGrid::processFromCell`).
    ///
    /// The order decides which of two equally good ends is taken, and
    /// which is taken when two are within the snap distance. Cells go row
    /// by row, from the lowest y and in each row from the lowest x, as in
    /// the reference. Within a cell the reference's order is that of an
    /// `unordered_multimap`'s equal keys, which the standard leaves open;
    /// here it is the order the ends were put in, so the lower line comes
    /// first and its first junction before its last. That is also what
    /// libc++ does (a new element goes after the ones equal to it), so it
    /// agrees with the reference built on a Mac.
    fn process_nearby(
        &self,
        query: Point2,
        radius: Coord,
        mut process: impl FnMut(LineEnd) -> bool,
    ) -> bool {
        let min = (
            self.to_grid_coord(query.x - radius),
            self.to_grid_coord(query.y - radius),
        );
        let max = (
            self.to_grid_coord(query.x + radius),
            self.to_grid_coord(query.y + radius),
        );
        for grid_y in min.1..=max.1 {
            for grid_x in min.0..=max.0 {
                let Some(ends) = self.cells.get(&(grid_x, grid_y)) else {
                    continue;
                };
                for &end in ends {
                    if !process(end) {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Whether a line may be turned round: only one down the middle, with no
/// outline beside it to keep on one hand (`canReverse`).
fn can_reverse(line: &ExtrusionLine) -> bool {
    line.is_odd
}

/// Whether two lines may be joined: not one down the middle with one
/// along an outline (`canConnect`).
fn can_connect(a: &ExtrusionLine, b: &ExtrusionLine) -> bool {
    a.is_odd == b.is_odd
}

/// `isOdd`.
fn is_odd(line: &ExtrusionLine) -> bool {
    line.is_odd
}

/// Stitches `lines` together: the pieces that end up as loops are added
/// to `result_polygons`, the rest to `result_lines`.
///
/// Starting from each line not yet used, in order, the nearest end of
/// another line within `max_stitch_distance` of this one's end is looked
/// for, and that line added on, until none is found or the end is back
/// within `snap_distance` of the start; then the same from the other end.
/// A join over less than `snap_distance` drops the junction joined to,
/// a longer one keeps both, with a new segment between. Nothing shorter
/// than three times `max_stitch_distance`, and nothing of two junctions,
/// is closed. A line with no junctions is left out (the reference has no
/// defined behaviour for one).
///
/// The result depends only on the lines and their order.
pub fn stitch(
    lines: &[ExtrusionLine],
    result_lines: &mut VariableWidthLines,
    result_polygons: &mut VariableWidthLines,
    max_stitch_distance: Coord,
    snap_distance: Coord,
) {
    if lines.is_empty() {
        return;
    }

    // SquareGrid asserts on its cell size; a reach of nothing or less
    // joins nothing, and a cell of one unit keeps the division defined.
    debug_assert!(max_stitch_distance > 0);
    let mut grid = EndGrid::new(max_stitch_distance.max(1), lines.len() * 2);

    for (line_idx, line) in lines.iter().enumerate() {
        let (Some(first), Some(last)) = (line.junctions.first(), line.junctions.last()) else {
            continue;
        };
        grid.insert(
            first.p,
            LineEnd {
                line_idx,
                point_idx: 0,
            },
        );
        grid.insert(
            last.p,
            LineEnd {
                line_idx,
                point_idx: line.junctions.len() - 1,
            },
        );
    }

    let mut processed = vec![false; lines.len()];

    for (line_idx, line) in lines.iter().enumerate() {
        if processed[line_idx] {
            continue;
        }
        processed[line_idx] = true;
        if line.junctions.is_empty() {
            continue;
        }
        // As the reference has it: only a chain that starts as an odd
        // line, and only until its first join, prefers closing.
        let mut should_close = is_odd(line);

        let mut chain = line.clone();
        let mut closest_is_closing_polygon = false;
        // Forward first, which may save turning the chain round. Only
        // this order works: the chain is turned back only when it closes.
        for go_in_reverse_direction in [false, true] {
            if go_in_reverse_direction {
                // Try to carry on from the other end.
                chain.reverse();
            }
            let mut chain_length = chain.get_length();

            loop {
                let from = chain.junctions[chain.junctions.len() - 1].p;
                let chain_front = chain.junctions[0].p;

                let mut closest: Option<LineEnd> = None;
                let mut closest_distance = Coord::MAX;
                grid.process_nearby(from, max_stitch_distance, |nearby| {
                    let nearby_line = &lines[nearby.line_idx];
                    let nearby_p = nearby_line.junctions[nearby.point_idx].p;
                    let mut is_closing_segment = false;
                    let mut dist = v_size(nearby_p, from);
                    if dist > max_stitch_distance {
                        return true; // keep looking
                    }
                    if v_size2(nearby_p, chain_front) < snap_distance * snap_distance {
                        // Too short to close (it may yet grow into a
                        // longer line), or only two junctions.
                        if chain_length + dist < 3 * max_stitch_distance
                            || chain.junctions.len() <= 2
                        {
                            return true; // look for a better next line
                        }
                        is_closing_segment = true;
                        if should_close {
                            // Prefer closing. Go on looking all the same.
                            dist -= CLOSING_PREFERENCE;
                        } else {
                            // Prefer carrying on to closing, so a closed
                            // zigzag is not printed apart. Go on looking:
                            // another end may close the loop better.
                            dist += CLOSING_PREFERENCE;
                        }
                    } else if processed[nearby.line_idx] {
                        // Already in the output.
                        return true;
                    }
                    // Seen from the chain's other end the line would be
                    // turned the other way.
                    let nearby_would_be_reversed =
                        (nearby.point_idx != 0) != go_in_reverse_direction;
                    if !can_reverse(nearby_line) && nearby_would_be_reversed {
                        return true;
                    }
                    if !can_connect(&chain, nearby_line) {
                        return true;
                    }
                    if dist < closest_distance {
                        closest_distance = dist;
                        closest = Some(nearby);
                        closest_is_closing_polygon = is_closing_segment;
                    }
                    // Near enough: stop looking for another.
                    dist >= snap_distance
                });

                let Some(closest) = closest else {
                    break; // no next line
                };
                if closest_is_closing_polygon {
                    break; // the loop is closed
                }

                let closest_line = &lines[closest.line_idx];
                let segment_dist = v_size(from, closest_line.junctions[closest.point_idx].p);
                debug_assert!(segment_dist <= max_stitch_distance + CLOSING_PREFERENCE);
                let old_size = chain.junctions.len();
                // Joined as good as end on end, the junction joined to is
                // left out.
                let skip = usize::from(segment_dist < snap_distance);
                if closest.point_idx == 0 {
                    chain
                        .junctions
                        .extend(closest_line.junctions.iter().skip(skip));
                } else {
                    chain
                        .junctions
                        .extend(closest_line.junctions.iter().rev().skip(skip));
                }
                for i in old_size..chain.junctions.len() {
                    chain_length += v_size(chain.junctions[i].p, chain.junctions[i - 1].p);
                }
                // Joined to an odd line, closing is no longer preferred.
                should_close &= !is_odd(closest_line);
                debug_assert!(!processed[closest.line_idx]);
                processed[closest.line_idx] = true;
            }

            if closest_is_closing_polygon {
                if go_in_reverse_direction {
                    // Back the way the first line ran.
                    chain.reverse();
                }
                break; // no need for the other direction
            }
        }

        if closest_is_closing_polygon {
            result_polygons.push(chain);
        } else {
            // Not closed, so both directions were tried and the chain is
            // turned round. One that may not be is turned back.
            if !can_reverse(line) {
                chain.reverse();
            }
            result_lines.push(chain);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Junction;

    /// A millimetre in units.
    const MM: i64 = 1_000_000;
    const MAX: Coord = DEFAULT_MAX_STITCH_DISTANCE;
    const SNAP: Coord = DEFAULT_SNAP_DISTANCE;

    fn line(inset_idx: usize, is_odd: bool, points: &[(i64, i64)]) -> ExtrusionLine {
        ExtrusionLine {
            inset_idx,
            is_odd,
            is_closed: false,
            junctions: points
                .iter()
                .enumerate()
                .map(|(i, &(x, y))| Junction {
                    p: Point2 { x, y },
                    w: 400_000 + i as i64,
                    perimeter_index: inset_idx,
                })
                .collect(),
        }
    }

    fn points(line: &ExtrusionLine) -> Vec<(i64, i64)> {
        line.junctions.iter().map(|j| (j.p.x, j.p.y)).collect()
    }

    fn run(lines: &[ExtrusionLine]) -> (VariableWidthLines, VariableWidthLines) {
        let (mut open, mut closed) = (Vec::new(), Vec::new());
        stitch(lines, &mut open, &mut closed, MAX, SNAP);
        (open, closed)
    }

    #[test]
    fn nothing_in_nothing_out_and_results_are_added_to() {
        let (open, closed) = run(&[]);
        assert!(open.is_empty() && closed.is_empty());

        let single = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let mut open = vec![line(3, true, &[(1, 1)])];
        let mut closed = Vec::new();
        stitch(
            std::slice::from_ref(&single),
            &mut open,
            &mut closed,
            MAX,
            SNAP,
        );
        assert_eq!(open.len(), 2);
        assert_eq!(open[1], single);
        assert!(closed.is_empty());
    }

    #[test]
    fn two_lines_within_reach_become_one() {
        // A gap of 0.05 mm: over the snap distance, so both ends stay.
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, false, &[(10 * MM + 50_000, 0), (20 * MM, 0)]);
        let (open, closed) = run(&[a.clone(), b.clone()]);
        assert!(closed.is_empty());
        assert_eq!(open.len(), 1);
        assert_eq!(
            points(&open[0]),
            vec![(0, 0), (10 * MM, 0), (10 * MM + 50_000, 0), (20 * MM, 0)]
        );
        // Widths and the rest come along with the junctions.
        assert_eq!(open[0].junctions[..2], a.junctions[..]);
        assert_eq!(open[0].junctions[2..], b.junctions[..]);
        assert!(!open[0].is_closed && !open[0].is_odd);

        // The later line first: found from the other end, same result.
        let (open, _) = run(&[b, a]);
        assert_eq!(open.len(), 1);
        assert_eq!(
            points(&open[0]),
            vec![(0, 0), (10 * MM, 0), (10 * MM + 50_000, 0), (20 * MM, 0)]
        );
    }

    #[test]
    fn a_join_within_the_snap_distance_drops_the_junction_joined_to() {
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, false, &[(10 * MM + 5_000, 0), (20 * MM, 0)]);
        let (open, _) = run(&[a, b]);
        assert_eq!(open.len(), 1);
        assert_eq!(points(&open[0]), vec![(0, 0), (10 * MM, 0), (20 * MM, 0)]);
    }

    #[test]
    fn lines_out_of_reach_stay_apart() {
        // 0.1 mm exactly is still within reach, one unit more is not.
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, false, &[(10 * MM + MAX, 0), (20 * MM, 0)]);
        assert_eq!(run(&[a.clone(), b]).0.len(), 1);
        let b = line(0, false, &[(10 * MM + MAX + 1, 0), (20 * MM, 0)]);
        let (open, closed) = run(&[a.clone(), b.clone()]);
        assert!(closed.is_empty());
        assert_eq!(open, vec![a, b]);
    }

    #[test]
    fn an_even_line_is_never_turned_round() {
        // b runs toward a's end: joining would turn one of them round.
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, false, &[(20 * MM, 0), (10 * MM + 50_000, 0)]);
        let (open, closed) = run(&[a.clone(), b.clone()]);
        assert!(closed.is_empty());
        assert_eq!(open, vec![a, b]);
    }

    #[test]
    fn odd_lines_are_joined_either_way_round_and_come_out_turned() {
        // The same two as odd lines: b is taken from its last junction.
        // Nothing closed, so the chain is left as the search from the
        // other end turned it.
        let a = line(0, true, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, true, &[(20 * MM, 0), (10 * MM + 50_000, 0)]);
        let (open, closed) = run(&[a.clone(), b]);
        assert!(closed.is_empty());
        assert_eq!(open.len(), 1);
        assert_eq!(
            points(&open[0]),
            vec![(20 * MM, 0), (10 * MM + 50_000, 0), (10 * MM, 0), (0, 0)]
        );
        assert!(open[0].is_odd);

        // An odd line on its own comes out turned round as well.
        let (open, _) = run(std::slice::from_ref(&a));
        assert_eq!(points(&open[0]), vec![(10 * MM, 0), (0, 0)]);
    }

    #[test]
    fn odd_and_even_lines_are_not_joined() {
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, true, &[(10 * MM, 0), (20 * MM, 0)]);
        let (open, closed) = run(&[a.clone(), b.clone()]);
        assert!(closed.is_empty());
        assert_eq!(open.len(), 2);
        assert_eq!(open[0], a);
        // The odd one turned round, as above.
        assert_eq!(points(&open[1]), vec![(20 * MM, 0), (10 * MM, 0)]);
    }

    #[test]
    fn inset_idx_is_not_looked_at() {
        // The reference's canConnect compares is_odd only; its caller
        // stitches one wall at a time.
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(1, false, &[(10 * MM, 0), (20 * MM, 0)]);
        let (open, _) = run(&[a, b]);
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].inset_idx, 0);
        assert_eq!(points(&open[0]), vec![(0, 0), (10 * MM, 0), (20 * MM, 0)]);
        // The junction that came from b says so still.
        assert_eq!(open[0].junctions[2].perimeter_index, 1);
    }

    fn square_sides() -> Vec<ExtrusionLine> {
        vec![
            line(0, false, &[(0, 0), (10 * MM, 0)]),
            line(0, false, &[(10 * MM, 0), (10 * MM, 10 * MM)]),
            line(0, false, &[(10 * MM, 10 * MM), (0, 10 * MM)]),
            line(0, false, &[(0, 10 * MM), (0, 0)]),
        ]
    }

    #[test]
    fn four_sides_of_a_square_close_into_a_polygon() {
        let (open, closed) = run(&square_sides());
        assert!(open.is_empty());
        assert_eq!(closed.len(), 1);
        // Each join is end on end, so each corner is there once; the last
        // side ends on the start, which is therefore there twice.
        assert_eq!(
            points(&closed[0]),
            vec![
                (0, 0),
                (10 * MM, 0),
                (10 * MM, 10 * MM),
                (0, 10 * MM),
                (0, 0)
            ]
        );
        // Left for the caller to set.
        assert!(!closed[0].is_closed);
    }

    #[test]
    fn a_square_closes_from_any_side_and_with_ends_a_little_apart() {
        // Starting with the third side, and with the last side ending
        // 0.005 mm short of the first one's start.
        let mut sides = square_sides();
        sides[3] = line(0, false, &[(0, 10 * MM), (0, 5_000)]);
        sides.rotate_left(2);
        let (open, closed) = run(&sides);
        assert!(open.is_empty());
        assert_eq!(closed.len(), 1);
        assert_eq!(
            points(&closed[0]),
            vec![
                (10 * MM, 10 * MM),
                (0, 10 * MM),
                (0, 5_000),
                (10 * MM, 0),
                (10 * MM, 10 * MM)
            ]
        );
    }

    #[test]
    fn a_line_back_at_its_own_start_closes_unless_it_is_too_small() {
        // Three junctions, 20 mm long, ends 0.005 mm apart: closed, as
        // it is.
        let a = line(0, false, &[(0, 0), (10 * MM, 0), (0, 5_000)]);
        let (open, closed) = run(std::slice::from_ref(&a));
        assert!(open.is_empty());
        assert_eq!(closed, vec![a]);

        // Two junctions are never closed.
        let a = line(0, false, &[(0, 0), (0, 5_000)]);
        let (open, closed) = run(std::slice::from_ref(&a));
        assert!(closed.is_empty());
        assert_eq!(open, vec![a]);

        // Nor is anything shorter than three times the reach: 0.08 mm
        // out, 0.08 mm up and back to 0.005 mm from the start is
        // 80000 + 80000 + 109658, and with the 5000 left to close still
        // under 300000.
        let a = line(
            0,
            false,
            &[(0, 0), (80_000, 0), (80_000, 80_000), (0, 5_000)],
        );
        let (open, closed) = run(std::slice::from_ref(&a));
        assert!(closed.is_empty());
        assert_eq!(open, vec![a]);
    }

    #[test]
    fn the_nearest_end_is_taken_and_a_tie_goes_to_the_lower_line() {
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let near = line(0, false, &[(10 * MM + 30_000, 0), (20 * MM, 0)]);
        let far = line(0, false, &[(10 * MM, 50_000), (10 * MM, 10 * MM)]);
        // The nearer one wins wherever it is in the list.
        for lines in [
            vec![a.clone(), near.clone(), far.clone()],
            vec![a.clone(), far.clone(), near.clone()],
        ] {
            let (open, _) = run(&lines);
            assert_eq!(open.len(), 2);
            assert_eq!(
                points(&open[0]),
                vec![(0, 0), (10 * MM, 0), (10 * MM + 30_000, 0), (20 * MM, 0)]
            );
            assert_eq!(open[1], far);
        }

        // Both 0.05 mm away and in one cell of the grid: the one earlier
        // in the list is taken.
        let right = line(0, false, &[(10 * MM + 50_000, 0), (20 * MM, 0)]);
        let up = far;
        let (open, _) = run(&[a.clone(), right.clone(), up.clone()]);
        assert_eq!(
            points(&open[0]),
            vec![(0, 0), (10 * MM, 0), (10 * MM + 50_000, 0), (20 * MM, 0)]
        );
        let (open, _) = run(&[a, up, right]);
        assert_eq!(
            points(&open[0]),
            vec![(0, 0), (10 * MM, 0), (10 * MM, 50_000), (10 * MM, 10 * MM)]
        );
    }

    #[test]
    fn the_first_end_within_the_snap_distance_stops_the_search() {
        // Both ends are within the snap distance of a's end and in one
        // cell. The first one seen is taken though the second is nearer.
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let b = line(0, false, &[(10 * MM + 8_000, 0), (20 * MM, 0)]);
        let c = line(0, false, &[(10 * MM + 2_000, 0), (10 * MM, 10 * MM)]);
        let (open, _) = run(&[a, b, c.clone()]);
        assert_eq!(open.len(), 2);
        assert_eq!(points(&open[0]), vec![(0, 0), (10 * MM, 0), (20 * MM, 0)]);
        assert_eq!(open[1], c);
    }

    #[test]
    fn ends_are_found_in_neighbouring_cells() {
        // The grid's cells are 0.1 mm: a's end is in cell -1 along x,
        // b's start, 92195 away, in cell 0 (which, the division cutting
        // toward zero, runs from -0.1 mm to 0.1 mm).
        let a = line(0, false, &[(-10 * MM, 0), (-120_000, 0)]);
        let b = line(0, false, &[(-30_000, 20_000), (10 * MM, 20_000)]);
        let (open, _) = run(&[a, b]);
        assert_eq!(open.len(), 1);
        assert_eq!(
            points(&open[0]),
            vec![
                (-10 * MM, 0),
                (-120_000, 0),
                (-30_000, 20_000),
                (10 * MM, 20_000)
            ]
        );
    }

    #[test]
    fn a_line_with_no_junctions_is_left_out() {
        let a = line(0, false, &[(0, 0), (10 * MM, 0)]);
        let (open, closed) = run(&[line(0, false, &[]), a.clone()]);
        assert!(closed.is_empty());
        assert_eq!(open, vec![a]);
    }

    #[test]
    fn the_same_lines_in_the_same_order_give_the_same_result() {
        // Two squares' worth of sides, a few stray pieces and a tie, in
        // an order that makes the search hop about.
        let mut lines = square_sides();
        lines.extend(square_sides().into_iter().map(|mut l| {
            for j in &mut l.junctions {
                j.p.x += 30 * MM;
            }
            l
        }));
        lines.push(line(0, true, &[(5 * MM, 5 * MM), (6 * MM, 5 * MM)]));
        lines.push(line(
            0,
            true,
            &[(6 * MM + 50_000, 5 * MM), (7 * MM, 5 * MM)],
        ));
        lines.push(line(
            0,
            true,
            &[(6 * MM, 5 * MM + 50_000), (6 * MM, 7 * MM)],
        ));
        lines.swap(1, 6);
        lines.swap(0, 9);

        let first = run(&lines);
        for _ in 0..20 {
            assert_eq!(run(&lines), first);
        }
        assert_eq!(first.1.len(), 2);
        assert_eq!(first.0.len(), 2);
    }
}
