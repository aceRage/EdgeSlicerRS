//! How much of a wall rests on the layer beneath, measured along it, so
//! the parts that hang out can be printed slower, cooled harder, and where
//! they hang clear, printed as bridges.
//!
//! The reference measures the same thing in ExtrusionProcessor.hpp: each
//! point's distance from the outline of the layer below, turned into how
//! much of the line's width has something under it. This is that scheme,
//! not that code: a wall is probed every 2 mm and again wherever its
//! speed, fan or kind changes, where the reference bisects and prunes.

use std::cell::Cell;

use edge_geom::{ExPolygon, Point2, scaled, unscaled};

/// A wall is probed at least this often along its length, in mm.
const PROBE: f64 = 2.0;
/// A stretch whose two ends differ is halved until it is this short, in mm.
const FINE: f64 = 0.25;

/// The outline of the layer beneath, laid on a grid so the edges near a
/// point are found without looking at them all.
pub(crate) struct Below {
    /// Each edge as start x, start y, end x, end y, in mm. Material is on
    /// the left of every edge: contours run counter-clockwise and holes
    /// clockwise.
    edges: Vec<[f64; 4]>,
    /// Further than this many mm from the outline a point is simply
    /// inside or outside. Cells are this size, so the edges within reach
    /// of a point are all in its cell or the eight round it.
    reach: f64,
    min: (f64, f64),
    columns: usize,
    rows: usize,
    /// The edges whose bounding box touches each cell.
    cells: Vec<Vec<u32>>,
    /// The edges that cross each row of cells, for counting crossings.
    bands: Vec<Vec<u32>>,
    /// For a cell with no edge in it or round it, whether it is over the
    /// layer beneath, once that has been asked: 0 not yet, 1 over it, 2 not.
    far: Vec<Cell<u8>>,
}

impl Below {
    /// None when there is nothing beneath. `reach` is as far, in mm, as
    /// any distance asked for needs to be told apart.
    pub(crate) fn new(regions: &[ExPolygon], reach: f64) -> Option<Self> {
        let mut edges = Vec::new();
        for ring in regions.iter().flat_map(ExPolygon::rings) {
            let points = &ring.points;
            for (index, from) in points.iter().enumerate() {
                let to = points[(index + 1) % points.len()];
                if *from != to {
                    edges.push([
                        unscaled(from.x),
                        unscaled(from.y),
                        unscaled(to.x),
                        unscaled(to.y),
                    ]);
                }
            }
        }
        if edges.is_empty() {
            return None;
        }
        let (mut min, mut max) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for [ax, ay, bx, by] in &edges {
            min = (min.0.min(ax.min(*bx)), min.1.min(ay.min(*by)));
            max = (max.0.max(ax.max(*bx)), max.1.max(ay.max(*by)));
        }
        let columns = ((max.0 - min.0) / reach) as usize + 1;
        let rows = ((max.1 - min.1) / reach) as usize + 1;
        let mut below = Self {
            edges,
            reach,
            min,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
            bands: vec![Vec::new(); rows],
            far: vec![Cell::new(0); columns * rows],
        };
        for index in 0..below.edges.len() {
            let [ax, ay, bx, by] = below.edges[index];
            let (left, right) = (below.column(ax.min(bx)), below.column(ax.max(bx)));
            let (low, high) = (below.row(ay.min(by)), below.row(ay.max(by)));
            for row in low..=high {
                below.bands[row].push(index as u32);
                for column in left..=right {
                    below.cells[row * columns + column].push(index as u32);
                }
            }
        }
        Some(below)
    }

    fn column(&self, x: f64) -> usize {
        (((x - self.min.0) / self.reach).max(0.0) as usize).min(self.columns - 1)
    }

    fn row(&self, y: f64) -> usize {
        (((y - self.min.1) / self.reach).max(0.0) as usize).min(self.rows - 1)
    }

    /// Distance in mm from a point to the outline: negative over the layer
    /// beneath, positive over air, and no further than the reach either way.
    pub(crate) fn signed_distance(&self, x: f64, y: f64) -> f64 {
        let reach = self.reach;
        let outside_grid = x < self.min.0 - reach
            || y < self.min.1 - reach
            || x > self.min.0 + (self.columns as f64 + 1.0) * reach
            || y > self.min.1 + (self.rows as f64 + 1.0) * reach;
        if outside_grid {
            return reach;
        }
        let (column, row) = (self.column(x), self.row(y));
        let cell = row * self.columns + column;
        match self.far[cell].get() {
            1 => return -reach,
            2 => return reach,
            _ => {}
        }
        // Nearest edge, and how far the point is from the line through it:
        // of two edges meeting at the nearest corner, the one the point
        // stands further off says which side the point is on.
        let (mut nearest, mut off_line, mut inside) = (f64::MAX, 0.0, false);
        for row in row.saturating_sub(1)..=(row + 1).min(self.rows - 1) {
            for column in column.saturating_sub(1)..=(column + 1).min(self.columns - 1) {
                for &index in &self.cells[row * self.columns + column] {
                    let [ax, ay, bx, by] = self.edges[index as usize];
                    let (dx, dy) = (bx - ax, by - ay);
                    let length_sq = dx * dx + dy * dy;
                    let along = (((x - ax) * dx + (y - ay) * dy) / length_sq).clamp(0.0, 1.0);
                    let distance = (x - ax - along * dx).hypot(y - ay - along * dy);
                    let cross = dx * (y - ay) - dy * (x - ax);
                    let off = cross.abs() / length_sq.sqrt();
                    if distance < nearest - 1e-9 || (distance < nearest + 1e-9 && off > off_line) {
                        (nearest, off_line, inside) = (nearest.min(distance), off, cross > 0.0);
                    }
                }
            }
        }
        if nearest <= reach {
            return if inside { -nearest } else { nearest };
        }
        // Too far from any edge to say by the nearest: count the edges a
        // ray from the point crosses.
        let mut inside = false;
        if y >= self.min.1 && y <= self.min.1 + self.rows as f64 * reach {
            for &index in &self.bands[row] {
                let [ax, ay, bx, by] = self.edges[index as usize];
                if (ay > y) != (by > y) && x < ax + (y - ay) / (by - ay) * (bx - ax) {
                    inside = !inside;
                }
            }
        }
        // With no edge in the cell or round it, the whole cell is alike.
        if nearest == f64::MAX {
            self.far[cell].set(if inside { 1 } else { 2 });
        }
        if inside { -reach } else { reach }
    }
}

/// One straight move of a wall, printed one way throughout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Piece {
    pub(crate) to: Point2,
    /// In mm/s, rounded to a whole number as the reference does.
    pub(crate) speed: f64,
    /// Whether so little of it is supported that it gets the overhang fan.
    pub(crate) cooled: bool,
    /// Whether it hangs clear of the layer beneath, and so is a bridge.
    pub(crate) clear: bool,
}

/// How a wall is to be judged.
pub(crate) struct Judge<'a> {
    pub(crate) below: &'a Below,
    /// Width of the wall's line in mm.
    pub(crate) width: f64,
    /// A line whose middle is further than this beyond the layer beneath
    /// hangs clear of it; none when clear stretches are not told apart.
    pub(crate) clear_beyond: Option<f64>,
    /// Speed in mm/s for a point this many mm short of full support.
    pub(crate) speed: &'a dyn Fn(f64) -> f64,
    /// The fan comes on where no more than this share of the width is
    /// supported; none when it does not go by that.
    pub(crate) fan_at: Option<f64>,
}

#[derive(Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
    /// Distance from the outline beneath, in mm; negative over it.
    signed: f64,
}

impl Judge<'_> {
    fn sample(&self, x: f64, y: f64) -> Sample {
        Sample {
            x,
            y,
            signed: self.below.signed_distance(x, y),
        }
    }

    /// How far a point is from having its whole width supported.
    fn short(&self, sample: &Sample) -> f64 {
        sample.signed + 0.5 * self.width
    }

    fn state(&self, sample: &Sample) -> (f64, bool, bool) {
        let short = self.short(sample);
        (
            (self.speed)(short),
            self.fan_at.is_some_and(|at| 1.0 - short / self.width <= at),
            self.clear_beyond
                .is_some_and(|beyond| sample.signed > beyond),
        )
    }

    /// The stretch between two samples, taken as the worse of its ends.
    fn piece(&self, from: &Sample, to: &Sample) -> Piece {
        let (a, b) = (self.state(from), self.state(to));
        Piece {
            to: Point2 {
                x: scaled(to.x),
                y: scaled(to.y),
            },
            speed: a.0.min(b.0),
            cooled: a.1 || b.1,
            clear: a.2 && b.2,
        }
    }

    /// Cuts the stretch between two samples wherever it changes, down to
    /// [`FINE`], and adds its pieces to `pieces`.
    fn refine(&self, from: Sample, to: Sample, pieces: &mut Vec<Piece>) {
        let length = (to.x - from.x).hypot(to.y - from.y);
        let differs = {
            let (a, b) = (self.state(&from), self.state(&to));
            (a.0 - b.0).abs() > 1.0 || a.1 != b.1 || a.2 != b.2
        };
        if differs && length > FINE {
            let middle = self.sample((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
            self.refine(from, middle, pieces);
            self.refine(middle, to, pieces);
        } else {
            pieces.push(self.piece(&from, &to));
        }
    }

    /// The moves that print the line through `points`, the first of which
    /// the nozzle is already at.
    pub(crate) fn pieces(&self, points: &[Point2]) -> Vec<Piece> {
        let mut pieces = Vec::with_capacity(points.len());
        let Some(first) = points.first() else {
            return pieces;
        };
        let mut from = self.sample(unscaled(first.x), unscaled(first.y));
        for point in &points[1..] {
            let end = self.sample(unscaled(point.x), unscaled(point.y));
            let length = (end.x - from.x).hypot(end.y - from.y);
            let steps = (length / PROBE).ceil().max(1.0) as usize;
            let start = pieces.len();
            let mut last = from;
            for step in 1..=steps {
                let next = if step == steps {
                    end
                } else {
                    let along = step as f64 / steps as f64;
                    self.sample(
                        from.x + (end.x - from.x) * along,
                        from.y + (end.y - from.y) * along,
                    )
                };
                self.refine(last, next, &mut pieces);
                last = next;
            }
            // Along one straight segment, neighbours printed alike are
            // one move.
            let mut kept = start;
            for index in start + 1..pieces.len() {
                let (next, held) = (pieces[index], pieces[kept]);
                if (next.speed, next.cooled, next.clear) == (held.speed, held.cooled, held.clear) {
                    pieces[kept].to = next.to;
                } else {
                    kept += 1;
                    pieces[kept] = next;
                }
            }
            pieces.truncate(kept + 1);
            // The segment ends exactly where it was given to.
            if let Some(last) = pieces.last_mut() {
                last.to = *point;
            }
            from = end;
        }
        pieces
    }
}

/// The speed for a point `short` mm from full support (ExtrusionProcessor.hpp
/// `calculate_speed`). `sections` pairs how far short a point is, as a
/// share of the line's width, with the speed there, nearest first; between
/// two the speed is interpolated, and it never exceeds `original`.
pub(crate) fn speed_for(sections: &[(f64, f64)], width: f64, original: f64, short: f64) -> f64 {
    let at = |index: usize| sections[index].0 * width;
    let last = sections.len() - 1;
    let speed = if short <= at(0) {
        original
    } else if short >= at(last) {
        sections[last].1
    } else {
        let mut index = 0;
        while short > at(index + 1) {
            index += 1;
        }
        let share = ((short - at(index)) / (at(index + 1) - at(index))).clamp(0.0, 1.0);
        (1.0 - share) * sections[index].1 + share * sections[index + 1].1
    };
    speed.round().min(original)
}

#[cfg(test)]
mod tests {
    use edge_geom::Polygon;

    use super::*;

    fn square(x: f64, y: f64, side: f64) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: Vec::new(),
        }
    }

    #[test]
    fn distance_is_negative_over_the_layer_beneath() {
        let mut plate = square(0.0, 0.0, 20.0);
        let mut hole = square(8.0, 8.0, 4.0).contour;
        hole.points.reverse();
        plate.holes.push(hole);
        let below = Below::new(&[plate], 2.0).unwrap();
        let near = |found: f64, wanted: f64| (found - wanted).abs() < 1e-9;
        assert!(near(below.signed_distance(1.0, 10.0), -1.0));
        assert!(near(below.signed_distance(-0.5, 10.0), 0.5));
        // In the hole there is nothing beneath.
        assert!(near(below.signed_distance(10.0, 10.0), 2.0));
        assert!(near(below.signed_distance(8.5, 10.0), 0.5));
        assert!(near(below.signed_distance(7.5, 10.0), -0.5));
        // Off a corner, where two edges are equally near.
        assert!(near(below.signed_distance(-0.3, -0.4), 0.5));
        assert!(below.signed_distance(0.3, 0.4) < 0.0);
        // Far away either side.
        assert_eq!(below.signed_distance(4.0, 4.0), -2.0);
        // Asked again, the cell remembers.
        assert_eq!(below.signed_distance(4.1, 4.2), -2.0);
        assert_eq!(below.signed_distance(40.0, 4.0), 2.0);
        assert_eq!(below.signed_distance(23.5, 4.0), 2.0);
        assert!(Below::new(&[], 2.0).is_none());
    }

    #[test]
    fn speed_falls_as_support_runs_out() {
        // Full speed to a tenth of the width short, then the profile's steps.
        let sections = [
            (0.1, 200.0),
            (0.25, 200.0),
            (0.5, 50.0),
            (0.75, 30.0),
            (0.87, 10.0),
            (1.0, 50.0),
        ];
        let speed = |short: f64| speed_for(&sections, 0.4, 200.0, short);
        assert_eq!(speed(-0.2), 200.0);
        assert_eq!(speed(0.04), 200.0);
        assert_eq!(speed(0.2), 50.0);
        assert_eq!(speed(0.25), 40.0);
        assert_eq!(speed(0.3), 30.0);
        assert_eq!(speed(0.348), 10.0);
        assert_eq!(speed(0.5), 50.0);
        // Never faster than the line was going to be printed anyway.
        assert_eq!(speed_for(&sections, 0.4, 25.0, 0.5), 25.0);
    }

    #[test]
    fn a_wall_is_cut_where_it_leaves_the_layer_beneath() {
        let below = Below::new(&[square(0.0, 0.0, 10.0)], 0.6).unwrap();
        let sections = [(0.1, 100.0), (0.5, 50.0), (1.0, 20.0)];
        let speed = |short: f64| speed_for(&sections, 0.4, 100.0, short);
        let judge = Judge {
            below: &below,
            width: 0.4,
            clear_beyond: Some(0.2),
            speed: &speed,
            fan_at: Some(0.5),
        };
        // From well inside straight out over the edge and on over air.
        let pieces = judge.pieces(&[Point2::from_mm(5.0, 5.0), Point2::from_mm(15.0, 5.0)]);
        assert!(pieces.len() >= 3, "{pieces:?}");
        let (first, last) = (pieces[0], pieces[pieces.len() - 1]);
        assert_eq!(
            (first.speed, first.cooled, first.clear),
            (100.0, false, false)
        );
        assert_eq!((last.speed, last.cooled, last.clear), (20.0, true, true));
        assert_eq!(last.to, Point2::from_mm(15.0, 5.0));
        // Supported until just short of the edge, clear soon after it.
        let supported_to = unscaled(first.to.x);
        assert!(supported_to > 9.0 && supported_to < 10.0, "{supported_to}");
        let clear_from = unscaled(pieces[pieces.len() - 2].to.x);
        assert!(clear_from > 10.0 && clear_from < 10.8, "{clear_from}");
        // Pieces run on from one another without going back.
        let ends: Vec<f64> = pieces.iter().map(|piece| unscaled(piece.to.x)).collect();
        assert!(ends.windows(2).all(|pair| pair[1] > pair[0]));

        // A wall wholly over the layer beneath is left as it was.
        let inside = judge.pieces(&[
            Point2::from_mm(2.0, 2.0),
            Point2::from_mm(8.0, 2.0),
            Point2::from_mm(8.0, 8.0),
        ]);
        assert_eq!(inside.len(), 2);
        assert!(
            inside
                .iter()
                .all(|piece| piece.speed == 100.0 && !piece.cooled)
        );
    }
}
