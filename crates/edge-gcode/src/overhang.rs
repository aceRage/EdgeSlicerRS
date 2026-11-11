//! How much of a wall rests on the layer beneath, measured along it, so
//! the parts that hang out can be printed slower, cooled harder, and where
//! they hang clear, printed as bridges.
//!
//! Ported from the reference's ExtrusionProcessor.hpp
//! (`estimate_points_properties` as
//! `ExtrusionQualityEstimator::estimate_extrusion_quality` calls it, and
//! that function itself): a wall is measured at its own points, at the
//! places where it crosses the outline beneath, at a few probes inside
//! its long lines and at up to two cuts beside the ends of a long line
//! that slows down. Each point then carries the speed and the cover of
//! the line that leaves it. No other point is added, so a wall of short
//! lines is written move for move as it would be with nothing measured.
//!
//! The reference works this out in single precision, and so does this:
//! a speed is rounded to a whole mm/s, and a point exactly half covered
//! turns the fan on or not by the last bit.
//!
//! Where a wall hangs clear it is another path in the reference, cut out
//! of the loop before any of this (PerimeterGenerator.cpp: the loop
//! against the layer beneath grown by half a nozzle, its corners
//! mitred). [`stretches`] makes that cut here, and each stretch is
//! measured by itself.
//!
//! Not ported: the slowing over lines of the layer beneath that are
//! expected to curl (`slowdown_for_curled_perimeters`, the reference's
//! `curled_lines`), which this engine does not estimate; that setting
//! only changes the speed of a line with nothing under it.

use std::cell::Cell;

use edge_geom::{ExPolygon, Point2, scaled, unscaled};

/// The reference's `EPSILON` (libslic3r.h), in mm.
const EPSILON: f64 = 1e-4;

/// The outline of the layer beneath, laid on a grid so the edges near a
/// point are found without looking at them all.
pub(crate) struct Below {
    /// Each edge as start x, start y, end x, end y, in mm. Material is on
    /// the left of every edge: contours run counter-clockwise and holes
    /// clockwise.
    edges: Vec<[f64; 4]>,
    /// For each edge, the edge that follows it round its ring.
    next: Vec<u32>,
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
        let mut next: Vec<u32> = Vec::new();
        for ring in regions.iter().flat_map(ExPolygon::rings) {
            let points = &ring.points;
            let first = edges.len();
            for (index, from) in points.iter().enumerate() {
                let to = points[(index + 1) % points.len()];
                if *from != to {
                    edges.push([
                        unscaled(from.x),
                        unscaled(from.y),
                        unscaled(to.x),
                        unscaled(to.y),
                    ]);
                    next.push(edges.len() as u32);
                }
            }
            if let Some(last) = next.last_mut().filter(|_| edges.len() > first) {
                *last = first as u32;
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
            next,
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

    /// As far as [`Self::signed_distance`] tells distances apart, in mm.
    pub(crate) fn reach(&self) -> f64 {
        self.reach
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

    /// [`Self::signed_distance`] told apart further, for a point where
    /// that is already known to be `near`: at any distance outside the
    /// outline, and as far as `inside` mm within it (beyond that,
    /// `-inside` or the reach, whichever is further). What the
    /// reference asks of its tree of lines (AABBTreeLines.hpp
    /// `distance_from_lines_extra`), which has no limit either way; a
    /// wall is not judged by anything deeper in than twice its width.
    fn distance_from(&self, near: f64, x: f64, y: f64, inside: f64) -> f64 {
        if near.abs() < self.reach {
            near
        } else if near > 0.0 {
            self.nearest(x, y, f64::INFINITY)
        } else if inside > self.reach {
            -self.nearest(x, y, inside)
        } else {
            near
        }
    }

    /// How far a point is from the nearest edge, looked for no further
    /// than `limit` mm: `limit` when there is none nearer.
    fn nearest(&self, x: f64, y: f64, limit: f64) -> f64 {
        let size = self.reach;
        // The cell the point is in, which may be off the grid.
        let column = ((x - self.min.0) / size).floor() as i64;
        let row = ((y - self.min.1) / size).floor() as i64;
        let (columns, rows) = (self.columns as i64, self.rows as i64);
        let mut best = limit;
        let mut ring = 0i64;
        // Every cell of a ring is at least one ring less than its number
        // of cells away from the point.
        while ((ring - 1) as f64) * size < best {
            let (left, right) = (column - ring, column + ring);
            let (low, high) = (row - ring, row + ring);
            if left < 0 && low < 0 && right >= columns && high >= rows {
                // The ring goes round the whole grid.
                break;
            }
            let mut look = |column: i64, row: i64| {
                for &index in &self.cells[row as usize * self.columns + column as usize] {
                    let [ax, ay, bx, by] = self.edges[index as usize];
                    let (dx, dy) = (bx - ax, by - ay);
                    let along =
                        (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                    best = best.min((x - ax - along * dx).hypot(y - ay - along * dy));
                }
            };
            for at in left.max(0)..=right.min(columns - 1) {
                if low >= 0 && low < rows {
                    look(at, low);
                }
                if high != low && high >= 0 && high < rows {
                    look(at, high);
                }
            }
            for at in (low + 1).max(0)..=(high - 1).min(rows - 1) {
                if left >= 0 && left < columns {
                    look(left, at);
                }
                if right != left && right >= 0 && right < columns {
                    look(right, at);
                }
            }
            ring += 1;
        }
        best
    }

    /// Where the line from `a` to `b` (in mm) meets the outline, as how
    /// far along the line each place is, from 0 at `a` to 1 at `b`,
    /// nearest `a` first. Two edges that meet where the line passes
    /// give that place twice. Lines whose directions' cross product is
    /// under `parallel` mm² are taken not to meet.
    fn hits(&self, a: (f64, f64), b: (f64, f64), parallel: f64) -> Vec<f64> {
        let reach = self.reach;
        let (left, right) = (self.column(a.0.min(b.0)), self.column(a.0.max(b.0)));
        let (low, high) = (self.row(a.1.min(b.1)), self.row(a.1.max(b.1)));
        // Off the grid altogether there is nothing to meet.
        let off = a.0.max(b.0) < self.min.0
            || a.1.max(b.1) < self.min.1
            || a.0.min(b.0) > self.min.0 + self.columns as f64 * reach
            || a.1.min(b.1) > self.min.1 + self.rows as f64 * reach;
        if off {
            return Vec::new();
        }
        let mut edges: Vec<u32> = Vec::new();
        for row in low..=high {
            for column in left..=right {
                edges.extend_from_slice(&self.cells[row * self.columns + column]);
            }
        }
        edges.sort_unstable();
        edges.dedup();
        // Line.hpp `line_alg::intersection`.
        let v1 = (b.0 - a.0, b.1 - a.1);
        let mut found: Vec<f64> = Vec::new();
        for index in edges {
            let [cx, cy, dx, dy] = self.edges[index as usize];
            let v2 = (dx - cx, dy - cy);
            let denom = v1.0 * v2.1 - v1.1 * v2.0;
            if denom.abs() < parallel || denom == 0.0 {
                continue;
            }
            let v12 = (a.0 - cx, a.1 - cy);
            let t1 = (v2.0 * v12.1 - v2.1 * v12.0) / denom;
            let t2 = (v1.0 * v12.1 - v1.1 * v12.0) / denom;
            if (0.0..=1.0).contains(&t1) && (0.0..=1.0).contains(&t2) {
                found.push(t1);
            }
        }
        // AABBTreeLines.hpp `get_intersections_with_line`, sorted.
        found.sort_by(f64::total_cmp);
        found
    }

    /// The corner that the outline grown by `by` mm has past the end of
    /// an edge, where the outline turns towards the material there: the
    /// corner's points in order, from the end of the edge's own line
    /// moved out to the start of the next edge's, and how many of the
    /// four there are. None where the outline turns the other way or
    /// goes straight on, and the two lines moved out meet or cross.
    ///
    /// As the reference grows an outline (ClipperUtils.cpp `offset`,
    /// which is clipper.cpp `ClipperOffset` with its mitre limit at 3):
    /// the two lines are carried on to where they meet (`DoMiter`),
    /// unless that is further from the corner than three times `by`,
    /// when the corner is cut across at `by` from it (`DoSquare`).
    fn corner(&self, index: usize, by: f64) -> Option<([(f64, f64); 4], usize)> {
        let [ax, ay, bx, by_] = self.edges[index];
        let [_, _, cx, cy] = self.edges[self.next[index] as usize];
        let unit = |dx: f64, dy: f64| {
            let length = dx.hypot(dy);
            (dx / length, dy / length)
        };
        let (u1, u2) = (unit(bx - ax, by_ - ay), unit(cx - bx, cy - by_));
        // Away from the material, which is on the left.
        let (n1, n2) = ((u1.1, -u1.0), (u2.1, -u2.0));
        let sin = n1.0 * n2.1 - n1.1 * n2.0;
        let cos = n1.0 * n2.0 + n1.1 * n2.1;
        if sin < 0.0 || (sin == 0.0 && cos > 0.0) {
            return None;
        }
        let from = (bx + by * n1.0, by_ + by * n1.1);
        let to = (bx + by * n2.0, by_ + by * n2.1);
        // "if (r >= m_miterLim) DoMiter(j, k, r); else DoSquare(j, k);"
        // with `m_miterLim = 2 / (MiterLimit * MiterLimit)`.
        if 1.0 + cos >= 2.0 / 9.0 {
            let reach = by / (1.0 + cos);
            let tip = (bx + (n1.0 + n2.0) * reach, by_ + (n1.1 + n2.1) * reach);
            Some(([from, tip, to, to], 3))
        } else {
            let along = by * (sin.atan2(cos) / 4.0).tan();
            Some((
                [
                    from,
                    (from.0 + along * u1.0, from.1 + along * u1.1),
                    (to.0 - along * u2.0, to.1 - along * u2.1),
                    to,
                ],
                4,
            ))
        }
    }

    /// The edges that touch the cells round a point, or a box of
    /// points, and those `margin` mm further out; none when the box is
    /// off the grid by more than that.
    fn edges_round(&self, low: (f64, f64), high: (f64, f64), margin: f64) -> Vec<u32> {
        let off = high.0 + margin < self.min.0
            || high.1 + margin < self.min.1
            || low.0 - margin > self.min.0 + self.columns as f64 * self.reach
            || low.1 - margin > self.min.1 + self.rows as f64 * self.reach;
        let mut edges: Vec<u32> = Vec::new();
        if off {
            return edges;
        }
        let (left, right) = (self.column(low.0 - margin), self.column(high.0 + margin));
        let (bottom, top) = (self.row(low.1 - margin), self.row(high.1 + margin));
        for row in bottom..=top {
            for column in left..=right {
                edges.extend_from_slice(&self.cells[row * self.columns + column]);
            }
        }
        edges.sort_unstable();
        edges.dedup();
        edges
    }

    /// Whether a point is outside the outline grown by `by` mm, corners
    /// as the reference grows them ([`Self::corner`]): what a line whose
    /// middle is there hangs clear by. `by` may be a third of the reach
    /// at most, which is as far as a corner goes.
    pub(crate) fn clear(&self, x: f64, y: f64, by: f64) -> bool {
        debug_assert!(3.0 * by <= self.reach + 1e-9);
        let away = self.signed_distance(x, y);
        if away <= by {
            // Within `by` of the outline, which the grown outline is
            // nowhere nearer than.
            return false;
        }
        if away >= self.reach {
            return true;
        }
        // Further than `by` from every edge, and still within the
        // grown outline if it is in one of its corners.
        !self
            .edges_round((x, y), (x, y), self.reach)
            .into_iter()
            .any(|index| {
                let Some((points, count)) = self.corner(index as usize, by) else {
                    return false;
                };
                // The corner's points and the outline's own corner are
                // a shape with no dents, gone round with the inside on
                // the left.
                let [_, _, bx, by_] = self.edges[index as usize];
                let mut from = (bx, by_);
                points[..count]
                    .iter()
                    .chain(std::iter::once(&(bx, by_)))
                    .all(|&to| {
                        let side = (to.0 - from.0) * (y - from.1) - (to.1 - from.1) * (x - from.0);
                        from = to;
                        side >= 0.0
                    })
            })
    }

    /// Where the line from `a` to `b` (in mm) may pass from one side of
    /// the outline grown by `by` mm to the other, as how far along the
    /// line each place is, in order: wherever it meets an edge moved
    /// out by `by` or a corner's line. Not every such place is on the
    /// grown outline, which is those lines' outermost run: [`Self::clear`]
    /// tells which side the parts between them are on.
    fn grown_hits(&self, a: (f64, f64), b: (f64, f64), by: f64) -> Vec<f64> {
        let low = (a.0.min(b.0), a.1.min(b.1));
        let high = (a.0.max(b.0), a.1.max(b.1));
        let mut found: Vec<f64> = Vec::new();
        let v1 = (b.0 - a.0, b.1 - a.1);
        let mut meet = |c: (f64, f64), d: (f64, f64)| {
            let v2 = (d.0 - c.0, d.1 - c.1);
            let denom = v1.0 * v2.1 - v1.1 * v2.0;
            if denom == 0.0 {
                return;
            }
            let v12 = (a.0 - c.0, a.1 - c.1);
            let t1 = (v2.0 * v12.1 - v2.1 * v12.0) / denom;
            let t2 = (v1.0 * v12.1 - v1.1 * v12.0) / denom;
            if t1 > 0.0 && t1 < 1.0 && (0.0..=1.0).contains(&t2) {
                found.push(t1);
            }
        };
        for index in self.edges_round(low, high, 3.0 * by) {
            let [cx, cy, dx, dy] = self.edges[index as usize];
            let length = (dx - cx).hypot(dy - cy);
            let out = (by * (dy - cy) / length, -by * (dx - cx) / length);
            meet((cx + out.0, cy + out.1), (dx + out.0, dy + out.1));
            if let Some((points, count)) = self.corner(index as usize, by) {
                for pair in points[..count].windows(2) {
                    meet(pair[0], pair[1]);
                }
            }
        }
        found.sort_by(f64::total_cmp);
        found.dedup();
        found
    }
}

/// A stretch of a wall that is over the layer beneath all the way, or
/// clear of it all the way.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stretch {
    pub(crate) points: Vec<Point2>,
    pub(crate) clear: bool,
}

/// A path cut where it leaves the layer beneath grown by `by` mm and
/// where it comes back over it, its stretches in the order they are
/// laid. A line whose middle is further out than that hangs clear, and
/// is an overhang wall; `by` is half the nozzle's width.
///
/// The reference makes the cut when the walls are made
/// (PerimeterGenerator.cpp `extrusion_paths_append` of `intersection_pl`
/// and `diff_pl` of the loop against `lower_polygons_series->back()`, or
/// with Arachne walls `clip_extrusion` against `lower_slices_polygons`:
/// in both, `offset(*this->lower_slices, scale_(nozzle_diameter / 2))`),
/// with its clipping library. Here the same places are found as the
/// path is laid, and without growing the outline: by where the path
/// meets the lines the grown outline is made of.
pub(crate) fn stretches(below: &Below, by: f64, points: &[Point2]) -> Vec<Stretch> {
    (stretches_measured(below, by, points).into_iter())
        .map(|(stretch, _)| stretch)
        .collect()
}

/// [`stretches`], each with the [`Below::signed_distance`] of its points,
/// which the cutting asks for and the measuring after it wants again.
pub(crate) fn stretches_measured(
    below: &Below,
    by: f64,
    points: &[Point2],
) -> Vec<(Stretch, Vec<f64>)> {
    let mut out: Vec<(Stretch, Vec<f64>)> = Vec::new();
    let Some(first) = points.first() else {
        return out;
    };
    let mm = |point: &Point2| (unscaled(point.x), unscaled(point.y));
    let mut away = below.signed_distance(unscaled(first.x), unscaled(first.y));
    for pair in points.windows(2) {
        let (a, b) = (mm(&pair[0]), mm(&pair[1]));
        let at = |along: f64| (a.0 + along * (b.0 - a.0), a.1 + along * (b.1 - a.1));
        let start_away = away;
        away = below.signed_distance(b.0, b.1);
        // A line too short to get from where its ends are to the grown
        // outline stays within it. Any other changes sides only where
        // it meets one of the lines that outline is made of, and which
        // side each part of it is on is asked at that part's middle.
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        let within = within(below, by, (a, start_away), (b, away), length, 3);
        let cuts = if within {
            Vec::new()
        } else {
            below.grown_hits(a, b, by)
        };
        let mut from = 0.0;
        for index in 0..=cuts.len() {
            let to = cuts.get(index).copied().unwrap_or(1.0);
            let state = !within && {
                let (x, y) = at((from + to) / 2.0);
                below.clear(x, y, by)
            };
            match out.last_mut() {
                Some((last, _)) if last.clear == state => {}
                last => {
                    // Where this part starts: the line's own start, or
                    // the cut before it.
                    let (start, start_away) = if index == 0 {
                        (pair[0], start_away)
                    } else {
                        let (x, y) = at(from);
                        let start = Point2 {
                            x: scaled(x),
                            y: scaled(y),
                        };
                        let away = below.signed_distance(unscaled(start.x), unscaled(start.y));
                        (start, away)
                    };
                    if let Some((last, aways)) = last
                        && last.points.last() != Some(&start)
                    {
                        last.points.push(start);
                        aways.push(start_away);
                    }
                    out.push((
                        Stretch {
                            points: vec![start],
                            clear: state,
                        },
                        vec![start_away],
                    ));
                }
            }
            from = to;
        }
        let (last, aways) = out.last_mut().expect("a line has at least one part");
        last.points.push(pair[1]);
        aways.push(away);
    }
    // A cut that lands on a point of the path leaves a stretch of no
    // length, which is no stretch; its neighbours are then one.
    out.retain(|(stretch, _)| {
        stretch
            .points
            .iter()
            .any(|point| *point != stretch.points[0])
    });
    let mut joined: Vec<(Stretch, Vec<f64>)> = Vec::with_capacity(out.len());
    for (stretch, aways) in out {
        match joined.last_mut() {
            Some((last, known)) if last.clear == stretch.clear => {
                last.points.extend_from_slice(&stretch.points[1..]);
                known.extend_from_slice(&aways[1..]);
            }
            _ => joined.push((stretch, aways)),
        }
    }
    if joined.is_empty() {
        // A path of no length is laid, or left out, as it was given.
        let stretch = Stretch {
            points: points.to_vec(),
            clear: below.clear(unscaled(first.x), unscaled(first.y), by),
        };
        let aways = (points.iter())
            .map(|point| below.signed_distance(unscaled(point.x), unscaled(point.y)))
            .collect();
        joined.push((stretch, aways));
    }
    joined
}

/// Whether the line between two points, each given with its
/// [`Below::signed_distance`], is sure to stay within the outline grown
/// by `by`: a point `t` mm along it is no further out than its start is
/// and `t` more, nor than its end is and what is left of its length.
/// A line too long to say so of is halved, `halvings` times at most.
fn within(
    below: &Below,
    by: f64,
    from: ((f64, f64), f64),
    to: ((f64, f64), f64),
    length: f64,
    halvings: u32,
) -> bool {
    if (from.1 + to.1 + length) / 2.0 <= by {
        return true;
    }
    // Nothing is known of a point at the reach or beyond but that.
    if halvings == 0 || from.1 > by || to.1 > by {
        return false;
    }
    let middle = ((from.0.0 + to.0.0) / 2.0, (from.0.1 + to.0.1) / 2.0);
    let middle = (middle, below.signed_distance(middle.0, middle.1));
    within(below, by, from, middle, length / 2.0, halvings - 1)
        && within(below, by, middle, to, length / 2.0, halvings - 1)
}

/// The speed of a line by how far it is short of full support
/// (ExtrusionProcessor.hpp `estimate_extrusion_quality`: `speed_sections`
/// and `calculate_speed`).
#[derive(Clone, Debug)]
pub(crate) struct Speeds {
    /// How far short of full support a line is, in mm, and its speed
    /// there in mm/s, nearest first; between two the speed is
    /// interpolated.
    sections: [(f32, f32); 6],
    /// The speed the line would be printed at anyway, which it never
    /// exceeds.
    original: f32,
}

/// The shares of a line's width that the six speeds are for, as the
/// width that has something under it, in percent (GCode.cpp `_extrude`,
/// `overhang_overlap_levels`).
const COVER: [f64; 6] = [90.0, 75.0, 50.0, 25.0, 13.0, 0.0];

impl Speeds {
    /// `steps` are the speeds in mm/s where a tenth, a quarter, a half,
    /// three quarters, 87% and all of the line's `width` has nothing
    /// under it; none is `reference` itself, the wall's own speed
    /// setting held to what the hot end can melt. The reference hands
    /// them over as percentages of that, and so are they taken here, so
    /// that they come out the same to the last bit. `original` is the
    /// speed the line is printed at where it is supported.
    pub(crate) fn new(width: f32, reference: f64, steps: [Option<f64>; 6], original: f64) -> Self {
        let mut sections = [(0.0f32, 0.0f32); 6];
        for (section, (cover, step)) in sections.iter_mut().zip(COVER.iter().zip(steps)) {
            let percent = step.map_or(100.0, |step| step * 100.0 / reference);
            *section = (
                (f64::from(width) * (1.0 - cover / 100.0)) as f32,
                (f64::from(reference as f32) * percent / 100.0) as f32,
            );
        }
        // The reference sorts the sections by distance here; they are
        // made in that order.
        Self {
            sections,
            original: original as f32,
        }
    }

    /// The speed in mm/s for a point `short` mm from full support,
    /// rounded to a whole number where it is not the line's own
    /// (`calculate_speed`).
    pub(crate) fn at(&self, short: f32) -> f32 {
        let sections = &self.sections;
        let speed = if short <= sections[0].0 {
            self.original
        } else if short >= sections[5].0 {
            sections[5].1
        } else {
            let mut index = 0;
            while short > sections[index + 1].0 {
                index += 1;
            }
            let share = ((short - sections[index].0) / (sections[index + 1].0 - sections[index].0))
                .clamp(0.0, 1.0);
            (1.0 - share) * sections[index].1 + share * sections[index + 1].1
        };
        speed.round()
    }

    /// [`Self::at`], and no more than the line's own speed
    /// (`effective_speed`).
    fn held(&self, short: f32) -> f32 {
        self.at(short).min(self.original)
    }

    /// The least a line has to be short of full support to slow down at
    /// all; none when no step is as slow as the line already is
    /// (`smallest_distance_with_lower_speed`).
    fn slows_from(&self) -> Option<f32> {
        self.sections
            .iter()
            .filter(|section| section.1 <= self.original)
            .map(|section| section.0)
            .reduce(f32::min)
    }
}

/// One point of a measured wall (ExtrusionProcessor.hpp `ProcessedPoint`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Reading {
    pub(crate) at: Point2,
    /// The speed in mm/s of the line from here to the next point: the
    /// slower of its two ends.
    pub(crate) speed: f32,
    /// The share of that line's width with something under it, at the
    /// worse of its two ends.
    pub(crate) cover: f32,
}

/// How a wall is to be measured.
pub(crate) struct Gauge<'a> {
    pub(crate) below: &'a Below,
    /// Width of the wall's line in mm.
    pub(crate) width: f32,
    pub(crate) speeds: &'a Speeds,
    /// The fan comes on where no more than this share of the width is
    /// supported; none when it does not go by that.
    pub(crate) fan_at: Option<f32>,
}

#[derive(Clone, Copy)]
struct Sample {
    x: f64,
    y: f64,
    /// How far the point is short of full support, in mm: its distance
    /// from the outline beneath (negative over it) and half the width.
    short: f32,
    /// The point of the path this is, when it is one.
    own: Option<Point2>,
}

impl Sample {
    fn to(&self, other: &Sample) -> f64 {
        (other.x - self.x).hypot(other.y - self.y)
    }
}

impl Gauge<'_> {
    fn half(&self) -> f32 {
        (0.5 * f64::from(self.width)) as f32
    }

    /// A place on the wall, measured.
    fn sample(&self, x: f64, y: f64, own: Option<Point2>) -> Sample {
        self.sample_from(self.below.signed_distance(x, y), x, y, own)
    }

    /// [`Self::sample`] of a place whose [`Below::signed_distance`] is
    /// already known to be `near`.
    fn sample_from(&self, near: f64, x: f64, y: f64, own: Option<Point2>) -> Sample {
        let away = self
            .below
            .distance_from(near, x, y, 2.0 * f64::from(self.width) + EPSILON);
        Sample {
            x,
            y,
            short: (away + f64::from(self.half())) as f32,
            own,
        }
    }

    /// Whether a point this far short of full support turns the
    /// overhang fan on (`fan_on`).
    fn fan_on(&self, short: f32) -> bool {
        self.fan_at
            .is_some_and(|at| 1.0 - short * (1.0 / self.width) <= at)
    }

    /// Whether two points print alike: within 1 mm/s of one another,
    /// which the writer does not tell apart, and with the fan the same
    /// (`same_speed_and_fan`).
    fn alike(&self, a: f32, b: f32) -> bool {
        (self.speeds.held(a) - self.speeds.held(b)).abs() <= 1.0 && self.fan_on(a) == self.fan_on(b)
    }

    /// Whether the first point prints slower than the second, or with
    /// the fan on where the second has it off (`slower_or_cooled`).
    fn worse(&self, a: f32, b: f32) -> bool {
        self.speeds.held(a) < self.speeds.held(b) - 1.0 || (self.fan_on(a) && !self.fan_on(b))
    }

    /// The points the wall through `path` is written by, each with how
    /// the line from it to the next is printed. The path's own points
    /// are all among them, in order. The [`Below::signed_distance`] of
    /// each of them may be `known` already (the reference's
    /// `known_distance`).
    pub(crate) fn readings(&self, path: &[Point2], known: Option<&[f64]>) -> Vec<Reading> {
        if path.is_empty() {
            return Vec::new();
        }
        let width = f64::from(self.width);
        let half = self.half();
        // "minimum spacing threshold for any newly generated points":
        // a quarter of the line's width.
        let spacing = width * 0.25;
        let own = |index: usize| {
            let point = path[index];
            let (x, y) = (unscaled(point.x), unscaled(point.y));
            match known {
                Some(known) => self.sample_from(known[index], x, y, Some(point)),
                None => self.sample(x, y, Some(point)),
            }
        };

        // The path's points, and where it crosses the outline beneath
        // ("Intersection handling").
        let over = |sample: &Sample| f64::from(sample.short) > f64::from(half) + EPSILON;
        let mut points: Vec<Sample> = Vec::with_capacity(path.len() * 3 / 2);
        points.push(own(0));
        for index in 1..path.len() {
            let next = own(index);
            let previous = points[points.len() - 1];
            if over(&previous) != over(&next) {
                // The reference keeps `previous` by reference while it
                // adds to the vector it is in (ExtrusionProcessor.hpp,
                // `const ExtendedPoint &prev_point = points.back()`),
                // and reads freed memory once the vector has grown; what
                // it means to compare with is this point.
                for along in self
                    .below
                    .hits((previous.x, previous.y), (next.x, next.y), EPSILON)
                {
                    let crossing = Sample {
                        x: previous.x + along * (next.x - previous.x),
                        y: previous.y + along * (next.y - previous.y),
                        short: half,
                        own: None,
                    };
                    if previous.to(&crossing) > spacing && crossing.to(&next) > spacing {
                        points.push(crossing);
                    }
                }
            }
            points.push(next);
        }

        // "Interior sampling": the two passes after this one go by a
        // line's ends alone, so a line supported at both ends and not
        // between them would read as supported. Its inside is probed,
        // and the probes kept that print differently from what is
        // either side of them. Skipped where nothing can be found: no
        // speed step slows this wall and the fan does not go by cover.
        if self.speeds.slows_from().is_some_and(|from| from > 0.0) || self.fan_at.is_some() {
            let probe = 2f64.max(4.0 * spacing);
            let mut sampled: Vec<Sample> = Vec::new();
            let mut inside: Vec<(f64, f32)> = Vec::new();
            let mut pending: Vec<(f64, f64, u32)> = Vec::new();
            for index in 0..points.len() - 1 {
                let (from, to) = (points[index], points[index + 1]);
                let length = from.to(&to);
                inside.clear();
                if length >= probe {
                    pending.push((0.0, 1.0, 10));
                }
                while let Some((start, end, depth)) = pending.pop() {
                    if depth == 0 || (end - start) * length < probe {
                        continue;
                    }
                    let middle = 0.5 * (start + end);
                    let found = self.sample(
                        from.x + middle * (to.x - from.x),
                        from.y + middle * (to.y - from.y),
                        None,
                    );
                    inside.push((middle, found.short));
                    pending.push((start, middle, depth - 1));
                    pending.push((middle, end, depth - 1));
                }
                if !inside.is_empty() {
                    inside.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let mut kept = 0;
                    for at in 0..inside.len() {
                        let short = inside[at].1;
                        let (at_start, at_end) = (kept == 0, at + 1 == inside.len());
                        let before = if at_start {
                            from.short
                        } else {
                            inside[kept - 1].1
                        };
                        let after = if at_end { to.short } else { inside[at + 1].1 };
                        // Differing from one of the line's own ends is
                        // not enough where the probe is not the worse
                        // of the two: the next pass confines an end's
                        // slowing to beside that end.
                        let worth_before =
                            !self.alike(short, before) && (!at_start || self.worse(short, before));
                        let worth_after =
                            !self.alike(short, after) && (!at_end || self.worse(short, after));
                        if worth_before || worth_after {
                            inside[kept] = inside[at];
                            kept += 1;
                        }
                    }
                    inside.truncate(kept);
                }
                if !inside.is_empty() && sampled.is_empty() {
                    sampled.extend_from_slice(&points[..=index]);
                }
                if !sampled.is_empty() {
                    for &(along, short) in &inside {
                        sampled.push(Sample {
                            x: from.x + along * (to.x - from.x),
                            y: from.y + along * (to.y - from.y),
                            short,
                            own: None,
                        });
                    }
                    sampled.push(to);
                }
            }
            if !sampled.is_empty() {
                points = sampled;
            }
        }

        // "Segmentation handling": a line prints as slow as its slower
        // end and is cooled if either end is, so one end over air would
        // slow the whole of a long line. A line of 2 mm or more with an
        // end that prints worse than a supported point is cut so that
        // only the part beside that end prints that way: as far from
        // the end as the end is short of support, and a line and a half
        // more.
        let mut cut_points: Vec<Sample> = Vec::with_capacity(points.len() * 2);
        cut_points.push(points[0]);
        for index in 0..points.len() - 1 {
            let (from, to) = (points[index], points[index + 1]);
            let near = |sample: &Sample| sample.short > -half && sample.short < half + 2.0;
            let length = from.to(&to);
            if (near(&from) || near(&to))
                && length >= 2.0
                && (self.worse(from.short, 0.0) || self.worse(to.short, 0.0))
            {
                let a0 = (f64::from(from.short + 3.0 * half) / length).clamp(0.0, 1.0);
                let a1 = (1.0 - f64::from(to.short + 3.0 * half) / length).clamp(0.0, 1.0);
                let ends = [a0.min(a1), a0.max(a1)];
                let mut cuts: [Option<Sample>; 2] = [None, None];
                let mut keep = [false, false];
                for which in 0..2 {
                    let along = ends[which];
                    // On the line's own end there is no cut.
                    if (which == 0 && along >= 1.0) || (which == 1 && along <= 0.0) {
                        continue;
                    }
                    let cut = self.sample(
                        from.x + along * (to.x - from.x),
                        from.y + along * (to.y - from.y),
                        None,
                    );
                    keep[which] = from.to(&cut) > spacing && cut.to(&to) > spacing;
                    cuts[which] = Some(cut);
                }
                // A cut stays only where it prints differently from the
                // point before it or the point after it.
                let short = |which: usize| cuts[which].map_or(0.0, |cut| cut.short);
                if keep[0] {
                    let after = if keep[1] { short(1) } else { to.short };
                    keep[0] = !self.alike(short(0), from.short) || !self.alike(short(0), after);
                }
                if keep[1] {
                    let before = if keep[0] { short(0) } else { from.short };
                    keep[1] = !self.alike(short(1), before) || !self.alike(short(1), to.short);
                }
                if let (true, true, Some(a), Some(b)) = (keep[0], keep[1], &cuts[0], &cuts[1]) {
                    // No scrap of a line between two cuts.
                    if a.to(b) <= spacing {
                        keep[1] = false;
                    }
                }
                for which in 0..2 {
                    if let (true, Some(cut)) = (keep[which], cuts[which]) {
                        cut_points.push(cut);
                    }
                }
            }
            cut_points.push(to);
        }
        let points = cut_points;

        let width_inv = 1.0 / self.width;
        (0..points.len())
            .map(|index| {
                let here = &points[index];
                let next = &points[(index + 1).min(points.len() - 1)];
                Reading {
                    at: here.own.unwrap_or(Point2 {
                        x: scaled(here.x),
                        y: scaled(here.y),
                    }),
                    speed: self
                        .speeds
                        .at(here.short)
                        .min(self.speeds.at(next.short))
                        .min(self.speeds.original),
                    cover: (1.0 - here.short * width_inv).min(1.0 - next.short * width_inv),
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use edge_geom::Polygon;

    use super::*;

    fn outline(points: &[(f64, f64)]) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
            },
            holes: Vec::new(),
        }
    }

    impl Below {
        fn distance(&self, x: f64, y: f64, inside: f64) -> f64 {
            self.distance_from(self.signed_distance(x, y), x, y, inside)
        }
    }

    fn square(x: f64, y: f64, side: f64) -> ExPolygon {
        outline(&[(x, y), (x + side, y), (x + side, y + side), (x, y + side)])
    }

    /// A line of 0.4 mm at 100 mm/s: full speed to a quarter of its
    /// width short of support, 50 mm/s at half, 30 at three quarters,
    /// 10 at 87% and 20 with nothing under it.
    fn speeds(original: f64) -> Speeds {
        Speeds::new(
            0.4,
            100.0,
            [None, None, Some(50.0), Some(30.0), Some(10.0), Some(20.0)],
            original,
        )
    }

    /// Where each reading is in mm along x and y, and its speed.
    fn summary(readings: &[Reading]) -> Vec<(f64, f64, f32)> {
        readings
            .iter()
            .map(|reading| {
                (
                    unscaled(reading.at.x),
                    unscaled(reading.at.y),
                    reading.speed,
                )
            })
            .collect()
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
    fn distance_is_told_apart_beyond_the_reach_where_it_is_asked_for() {
        // A reach of 0.6 mm, as the writer builds it, on a 20 mm plate.
        let below = Below::new(&[square(0.0, 0.0, 20.0)], 0.6).unwrap();
        let near = |found: f64, wanted: f64| (found - wanted).abs() < 1e-9;
        // Outside, however far: off the grid, off a corner, and close by.
        assert!(near(below.distance(23.5, 4.0, 0.9), 3.5));
        assert!(near(below.distance(120.0, 10.0, 0.9), 100.0));
        assert!(near(below.distance(23.0, 24.0, 0.9), 5.0));
        assert!(near(below.distance(-7.0, -24.0, 0.9), 25.0));
        assert!(near(below.distance(20.25, 4.0, 0.9), 0.25));
        // Inside, as deep as was asked for and no deeper.
        assert!(near(below.distance(0.3, 10.0, 0.9), -0.3));
        assert!(near(below.distance(0.8, 10.0, 0.9), -0.8));
        assert!(near(below.distance(10.0, 10.0, 0.9), -0.9));
        assert!(near(below.distance(4.0, 10.0, 7.5), -4.0));
        assert!(near(below.distance(10.0, 10.0, 30.0), -10.0));
        // With no more asked than the reach, the reach.
        assert!(near(below.distance(10.0, 10.0, 0.5), -0.6));
        // What it adds agrees with what was there within the reach.
        for step in 0..=80 {
            let x = 19.0 + 0.025 * step as f64;
            assert!(near(
                below.distance(x, 7.0, 0.9).clamp(-0.6, 0.6),
                below.signed_distance(x, 7.0)
            ));
        }
    }

    #[test]
    fn a_line_meets_the_outline_where_it_crosses_it() {
        let below = Below::new(&[square(0.0, 0.0, 20.0)], 0.6).unwrap();
        let near = |found: &[f64], wanted: &[f64]| {
            found.len() == wanted.len()
                && found.iter().zip(wanted).all(|(a, b)| (a - b).abs() < 1e-9)
        };
        assert!(near(&below.hits((5.0, 5.0), (25.0, 5.0), EPSILON), &[0.75]));
        // Right through, nearest the start first, whichever way.
        assert!(near(
            &below.hits((-10.0, 5.0), (30.0, 5.0), EPSILON),
            &[0.25, 0.75]
        ));
        assert!(near(
            &below.hits((30.0, 5.0), (-10.0, 5.0), EPSILON),
            &[0.25, 0.75]
        ));
        // Inside all the way, outside all the way, and off the grid.
        assert!(below.hits((5.0, 5.0), (15.0, 15.0), EPSILON).is_empty());
        assert!(below.hits((21.0, 5.0), (25.0, 15.0), EPSILON).is_empty());
        assert!(below.hits((50.0, 50.0), (60.0, 55.0), EPSILON).is_empty());
        // Along an edge the two never cross.
        assert!(below.hits((5.0, 20.0), (15.0, 20.0), EPSILON).is_empty());
    }

    #[test]
    fn speed_falls_as_support_runs_out() {
        let speed = |short: f32| speeds(100.0).at(short);
        assert_eq!(speed(-0.2), 100.0);
        assert_eq!(speed(0.03), 100.0);
        assert_eq!(speed(0.09), 100.0);
        assert_eq!(speed(0.15), 75.0);
        assert_eq!(speed(0.2), 50.0);
        assert_eq!(speed(0.25), 40.0);
        assert_eq!(speed(0.3), 30.0);
        assert_eq!(speed(0.348), 10.0);
        // A whole number of mm/s: 10 mm/s and 6/13 of the way to 20.
        assert_eq!(speed(0.372), 15.0);
        assert_eq!(speed(0.4), 20.0);
        assert_eq!(speed(5.0), 20.0);
        // A wall that was going slower anyway is not made faster, and
        // where it is supported it keeps its own speed, whole or not.
        let slow = speeds(25.4);
        assert_eq!(slow.at(0.0), 25.0);
        assert_eq!(slow.at(0.2), 50.0);
        assert_eq!(slow.held(0.2), 25.4);
        assert_eq!(slow.held(0.348), 10.0);
        // The least overhang that slows it: the first step no faster
        // than it goes already.
        let from = speeds(100.0).slows_from().unwrap();
        assert!((from - 0.04).abs() < 1e-6, "{from}");
        let from = slow.slows_from().unwrap();
        assert!((from - 0.348).abs() < 1e-6, "{from}");
        assert_eq!(speeds(5.0).slows_from(), None);
    }

    #[test]
    fn a_long_line_is_cut_beside_the_end_that_hangs_out() {
        // Straight out over the edge of a plate at x = 10 and on over
        // air. Worked by hand from ExtrusionProcessor.hpp:
        //
        // - (10, 5), where it crosses the outline: half its width short
        //   of support, 50 mm/s.
        // - (11.25, 5): the first of the probes inside the line from
        //   there on, kept because it prints differently from the
        //   crossing; the two after it print like it and are dropped.
        //   No probe inside the supported line is kept.
        // - (9.2, 5): the cut beside the end at the crossing, 0.2 mm
        //   (how far short that end is) and 0.6 mm (a line and a half)
        //   before it, so that only that much slows down. The cut
        //   beside the far end of the line over air prints like both
        //   its neighbours and is dropped.
        let below = Below::new(&[square(0.0, 0.0, 10.0)], 0.6).unwrap();
        let speeds = speeds(100.0);
        let gauge = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: None,
        };
        let path = [Point2::from_mm(5.0, 5.0), Point2::from_mm(15.0, 5.0)];
        let readings = gauge.readings(&path, None);
        assert_eq!(
            summary(&readings),
            [
                (5.0, 5.0, 100.0),
                (9.2, 5.0, 50.0),
                (10.0, 5.0, 20.0),
                (11.25, 5.0, 20.0),
                (15.0, 5.0, 20.0),
            ]
        );
        assert_eq!(readings[0].at, path[0]);
        assert_eq!(readings[4].at, path[1]);
        // Each point's cover is that of the worse end of the line
        // leaving it: the cut's is the crossing's half.
        assert!(readings[0].cover > 1.0);
        assert_eq!(readings[1].cover, 0.5);
        assert!(readings[2].cover < 0.0);

        // A wall wholly over the layer beneath is left as it was.
        let inside = [
            Point2::from_mm(2.0, 2.0),
            Point2::from_mm(8.0, 2.0),
            Point2::from_mm(8.0, 8.0),
        ];
        assert_eq!(
            summary(&gauge.readings(&inside, None)),
            [(2.0, 2.0, 100.0), (8.0, 2.0, 100.0), (8.0, 8.0, 100.0)]
        );
    }

    #[test]
    fn a_line_is_probed_inside_for_what_its_ends_do_not_show() {
        // A plate with a notch 4 mm wide in its near side, and a wall
        // straight along that side, supported at both ends. Probed
        // every millimetre (a line of 32 mm halved until its parts are
        // under 2 mm), it is found unsupported from 18.5 to 21.5: those
        // two probes stay, with the supported one either side of them,
        // and the two between them, which print like them, go.
        let plate = outline(&[
            (0.0, 0.0),
            (18.0, 0.0),
            (18.0, 3.0),
            (22.0, 3.0),
            (22.0, 0.0),
            (40.0, 0.0),
            (40.0, 10.0),
            (0.0, 10.0),
        ]);
        let below = Below::new(&[plate], 0.6).unwrap();
        let speeds = speeds(100.0);
        let gauge = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: None,
        };
        let path = [Point2::from_mm(4.5, 0.2), Point2::from_mm(36.5, 0.2)];
        assert_eq!(
            summary(&gauge.readings(&path, None)),
            [
                (4.5, 0.2, 100.0),
                (17.5, 0.2, 20.0),
                (18.5, 0.2, 20.0),
                (21.5, 0.2, 20.0),
                (22.5, 0.2, 100.0),
                (36.5, 0.2, 100.0),
            ]
        );

        // A line under 2 mm is not probed. Over a notch 1 mm wide, one
        // of 1.9 mm with both its ends supported is laid as if all of it
        // were; one of 2.2 mm is probed at its middle, found to hang
        // there, and slowed from end to end, since a line goes at the
        // slower of its ends and neither half is long enough to be cut.
        let plate = outline(&[
            (0.0, 0.0),
            (18.0, 0.0),
            (18.0, 3.0),
            (19.0, 3.0),
            (19.0, 0.0),
            (40.0, 0.0),
            (40.0, 10.0),
            (0.0, 10.0),
        ]);
        let below = Below::new(&[plate], 0.6).unwrap();
        let gauge = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: None,
        };
        let short = [Point2::from_mm(17.6, 0.2), Point2::from_mm(19.5, 0.2)];
        assert_eq!(
            summary(&gauge.readings(&short, None)),
            [(17.6, 0.2, 100.0), (19.5, 0.2, 100.0)]
        );
        let longer = [Point2::from_mm(17.4, 0.2), Point2::from_mm(19.6, 0.2)];
        assert_eq!(
            summary(&gauge.readings(&longer, None)),
            [(17.4, 0.2, 20.0), (18.5, 0.2, 20.0), (19.6, 0.2, 100.0)]
        );
    }

    #[test]
    fn a_wall_of_short_lines_gets_no_points_but_its_own() {
        // Along the plate's edge and nearer to it at every point, in
        // lines of half a millimetre.
        let below = Below::new(&[square(0.0, 0.0, 30.0)], 0.6).unwrap();
        let speeds = speeds(100.0);
        let gauge = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: Some(0.5),
        };
        let path: Vec<Point2> = (0..=40)
            .map(|step| Point2::from_mm(29.75 + 0.006 * step as f64, 5.0 + 0.5 * step as f64))
            .collect();
        let readings = gauge.readings(&path, None);
        assert_eq!(readings.len(), path.len());
        assert!(
            readings
                .iter()
                .zip(&path)
                .all(|(reading, point)| reading.at == *point)
        );
        // It starts with all its width supported, at full speed, and
        // ends 0.19 mm short: nine tenths of the way from 100 mm/s at a
        // quarter of its width to 50 mm/s at half.
        assert_eq!(readings[0].speed, 100.0);
        assert!(
            readings
                .windows(2)
                .all(|pair| pair[1].speed <= pair[0].speed)
        );
        assert_eq!(readings[40].speed, 55.0);
        // A point carries the speed of the line that leaves it, which
        // is that of the line's slower end.
        assert_eq!(readings[39].speed, 55.0);
        assert!(readings[38].speed > 55.0);
        // More than half of it is supported all the way: no fan.
        assert!(readings.iter().all(|reading| reading.cover > 0.5));
    }

    #[test]
    fn a_line_is_cut_where_the_fan_comes_on_though_its_speed_is_one() {
        // A wall going at 5 mm/s, slower than any overhang would make
        // it: its speed is the same all along, and it is still cut
        // where it crosses the outline and beside it, for the fan.
        let below = Below::new(&[square(0.0, 0.0, 10.0)], 0.6).unwrap();
        let speeds = speeds(5.0);
        let path = [Point2::from_mm(5.0, 5.0), Point2::from_mm(15.0, 5.0)];
        let cooled = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: Some(0.5),
        }
        .readings(&path, None);
        assert!(cooled.iter().all(|reading| reading.speed == 5.0));
        let ends: Vec<f64> = cooled
            .iter()
            .map(|reading| unscaled(reading.at.x))
            .collect();
        assert_eq!(ends, [5.0, 9.2, 10.0, 15.0]);
        assert!(cooled[0].cover > 0.5 && cooled[1].cover <= 0.5);
        // With no fan to turn on there is nothing to find inside the
        // line or beside its ends: only the crossing is added.
        let plain = Gauge {
            below: &below,
            width: 0.4,
            speeds: &speeds,
            fan_at: None,
        }
        .readings(&path, None);
        let ends: Vec<f64> = plain.iter().map(|reading| unscaled(reading.at.x)).collect();
        assert_eq!(ends, [5.0, 10.0, 15.0]);
    }

    #[test]
    fn a_path_is_cut_where_it_leaves_the_grown_outline_and_comes_back() {
        // A plate 10 mm square and a nozzle of 0.4 mm: a line hangs
        // clear from 0.2 mm beyond the plate's edge.
        let below = Below::new(&[square(0.0, 0.0, 10.0)], 0.6).unwrap();
        let at = |x: f64, y: f64| Point2::from_mm(x, y);
        let cut = |path: &[Point2]| stretches(&below, 0.2, path);
        // Out over one side, round, and back over it further up.
        let path = [at(5.0, 5.0), at(15.0, 5.0), at(15.0, 8.0), at(5.0, 8.0)];
        assert_eq!(
            cut(&path),
            [
                Stretch {
                    points: vec![at(5.0, 5.0), at(10.2, 5.0)],
                    clear: false,
                },
                Stretch {
                    points: vec![at(10.2, 5.0), at(15.0, 5.0), at(15.0, 8.0), at(10.2, 8.0)],
                    clear: true,
                },
                Stretch {
                    points: vec![at(10.2, 8.0), at(5.0, 8.0)],
                    clear: false,
                },
            ]
        );
        // Wholly over it, wholly clear of it, and with a point on the
        // grown outline.
        let over = [at(2.0, 2.0), at(8.0, 2.0), at(10.1, 8.0)];
        assert_eq!(
            cut(&over),
            [Stretch {
                points: over.to_vec(),
                clear: false,
            }]
        );
        let off = [at(12.0, 2.0), at(18.0, 2.0), at(18.0, 8.0)];
        assert_eq!(
            cut(&off),
            [Stretch {
                points: off.to_vec(),
                clear: true,
            }]
        );
        let through = [at(5.0, 5.0), at(10.2, 5.0), at(15.0, 5.0)];
        let parts = cut(&through);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].points, [at(5.0, 5.0), at(10.2, 5.0)]);
        assert_eq!(parts[1].points, [at(10.2, 5.0), at(15.0, 5.0)]);
        // Right across and out the other side: three stretches of one line.
        let across = [at(-5.0, 5.0), at(15.0, 5.0)];
        let parts = cut(&across);
        assert_eq!(
            parts
                .iter()
                .map(|stretch| stretch.clear)
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(parts[1].points, [at(-0.2, 5.0), at(10.2, 5.0)]);
        // A line that touches the grown outline's corner from outside
        // is not cut there.
        let past = [at(5.4, 15.0), at(15.0, 5.4)];
        assert_eq!(cut(&past).len(), 1);
        assert!(cut(&[]).is_empty());
    }

    #[test]
    fn the_grown_outline_has_the_corners_the_reference_gives_it() {
        // ClipperOffset with a mitre limit of 3: a square corner is
        // carried out to a point, 0.2 mm out on both sides, where going
        // by the distance from the plate would round it off at 0.2 mm
        // from the corner.
        let below = Below::new(&[square(0.0, 0.0, 10.0)], 0.6).unwrap();
        // 0.212 mm from the corner, and within the mitre.
        assert!(below.signed_distance(10.15, 10.15) > 0.2);
        assert!(!below.clear(10.15, 10.15, 0.2));
        assert!(!below.clear(10.199, 10.199, 0.2));
        assert!(below.clear(10.201, 10.199, 0.2));
        assert!(below.clear(10.199, 10.201, 0.2));
        // Along a side there is no more to it than the distance.
        assert!(!below.clear(10.199, 5.0, 0.2));
        assert!(below.clear(10.201, 5.0, 0.2));
        assert!(!below.clear(5.0, 5.0, 0.2));
        assert!(below.clear(13.0, 5.0, 0.2));
        // A line past the corner on the slant is cut where it meets the
        // mitre's two sides, though all of it is further than 0.2 mm
        // from the plate.
        let at = |x: f64, y: f64| Point2::from_mm(x, y);
        let parts = stretches(&below, 0.2, &[at(9.8, 10.5), at(10.5, 9.8)]);
        assert_eq!(
            parts
                .iter()
                .map(|stretch| stretch.clear)
                .collect::<Vec<_>>(),
            [true, false, true]
        );
        assert_eq!(parts[1].points, [at(10.1, 10.2), at(10.2, 10.1)]);
        // One that only touches the mitre's point is not cut.
        assert_eq!(
            stretches(&below, 0.2, &[at(9.9, 10.5), at(10.5, 9.9)]).len(),
            1
        );

        // A corner sharper than 39 degrees, whose mitre would reach
        // further than three times the distance, is cut across at that
        // distance from it: a spike 20 mm long and 2 mm wide at its
        // foot, pointing along x, grown by 0.2 mm.
        let spike = outline(&[(0.0, -1.0), (20.0, 0.0), (0.0, 1.0)]);
        let below = Below::new(&[spike], 0.6).unwrap();
        assert!(!below.clear(20.19, 0.0, 0.2));
        assert!(below.clear(20.21, 0.0, 0.2));
        // Beside the tip the cut runs square to the spike, out to the
        // lines of its two sides: 0.19 mm past the tip and 0.15 mm off
        // its middle is inside, where nothing of the spike is within
        // 0.2 mm.
        assert!(below.signed_distance(20.19, 0.15) > 0.2);
        assert!(!below.clear(20.19, 0.15, 0.2));
        assert!(below.clear(20.19, 0.25, 0.2));

        // Where the outline turns away from the material there is no
        // corner to add: inside the bend of an L, 0.25 mm from both
        // arms, a line hangs clear.
        let bend = outline(&[
            (0.0, 0.0),
            (10.0, 0.0),
            (10.0, 5.0),
            (5.0, 5.0),
            (5.0, 10.0),
            (0.0, 10.0),
        ]);
        let below = Below::new(&[bend], 0.6).unwrap();
        assert!(below.clear(5.25, 5.25, 0.2));
        assert!(!below.clear(5.15, 5.25, 0.2));
        assert!(!below.clear(5.25, 5.15, 0.2));
    }
}
