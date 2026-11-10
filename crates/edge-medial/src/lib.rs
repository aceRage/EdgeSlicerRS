//! The centre line of a narrow region, with how wide the region is along
//! it: what a gap too thin for a wall is filled with.
//!
//! A port of the reference's `Geometry::MedialAxis` (MedialAxis.cpp) and of
//! the clean-up in `ExPolygon::medial_axis` (ExPolygon.cpp). Both stand on
//! the Voronoi diagram of the region's edges; the reference uses
//! Boost.Polygon's and this crate a Rust port of it, and it is the only
//! crate that knows which.

mod thick;

use boostvoronoi::prelude::{Builder, BvError, Diagram, EdgeIndex, SourceCategory};
use edge_geom::{ExPolygon, Point2};

pub use thick::variable_width;

/// The reference's `SCALED_EPSILON`: lengths below it count as nothing.
const EPSILON: f64 = 100.0;

#[derive(Debug, thiserror::Error)]
pub enum MedialError {
    /// The diagram is exact only while coordinates fit in 32 bits, which
    /// is about two metres across.
    #[error("the region is too large for a Voronoi diagram")]
    TooLarge,
    #[error("the Voronoi diagram could not be built: {0}")]
    Voronoi(#[from] BvError),
}

/// A line whose width changes along it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThickPolyline {
    pub points: Vec<Point2>,
    /// The width at the start and at the end of each segment, in the units
    /// of the points: two values for every segment.
    pub widths: Vec<f64>,
    /// Whether each end is a free end, as opposed to a junction with
    /// other lines.
    pub open_ends: (bool, bool),
}

impl ThickPolyline {
    /// Length in the units of the points.
    pub fn length(&self) -> f64 {
        self.points
            .windows(2)
            .map(|pair| distance(as_f64(pair[0]), as_f64(pair[1])))
            .sum()
    }

    pub fn is_closed(&self) -> bool {
        self.points.len() > 2 && self.points.first() == self.points.last()
    }

    fn reverse(&mut self) {
        self.points.reverse();
        self.widths.reverse();
        self.open_ends = (self.open_ends.1, self.open_ends.0);
    }
}

type Vec2 = (f64, f64);

fn as_f64(point: Point2) -> Vec2 {
    (point.x as f64, point.y as f64)
}

fn distance(a: Vec2, b: Vec2) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// Distance from `point` to the segment from `a` to `b`.
fn distance_to_segment(point: Vec2, a: Vec2, b: Vec2) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_sq = dx * dx + dy * dy;
    if length_sq == 0.0 {
        return distance(point, a);
    }
    let along = (((point.0 - a.0) * dx + (point.1 - a.1) * dy) / length_sq).clamp(0.0, 1.0);
    distance(point, (a.0 + along * dx, a.1 + along * dy))
}

/// Where the segments `a` and `b` cross, if they do.
fn crossing(a: (Vec2, Vec2), b: (Vec2, Vec2)) -> Option<Vec2> {
    let (r, s) = (
        (a.1.0 - a.0.0, a.1.1 - a.0.1),
        (b.1.0 - b.0.0, b.1.1 - b.0.1),
    );
    let denominator = r.0 * s.1 - r.1 * s.0;
    if denominator == 0.0 {
        return None;
    }
    let between = (b.0.0 - a.0.0, b.0.1 - a.0.1);
    let t = (between.0 * s.1 - between.1 * s.0) / denominator;
    let u = (between.0 * r.1 - between.1 * r.0) / denominator;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u))
        .then_some((a.0.0 + t * r.0, a.0.1 + t * r.1))
}

/// The region's edges, each ring in turn, in coordinates measured from
/// `origin`.
struct Outline {
    origin: Point2,
    /// Start and end of every edge.
    edges: Vec<[i64; 4]>,
    /// How many of `edges` belong to the contour; the rest are holes'.
    contour: usize,
}

impl Outline {
    fn new(region: &ExPolygon) -> Result<Self, MedialError> {
        let all = || region.rings().flat_map(|ring| &ring.points);
        let origin = Point2 {
            x: all().map(|point| point.x).min().unwrap_or(0),
            y: all().map(|point| point.y).min().unwrap_or(0),
        };
        let limit = i64::from(i32::MAX / 2);
        if all().any(|point| point.x - origin.x > limit || point.y - origin.y > limit) {
            return Err(MedialError::TooLarge);
        }
        let mut edges = Vec::new();
        let mut contour = 0;
        for (index, ring) in region.rings().enumerate() {
            let points = &ring.points;
            for (at, from) in points.iter().enumerate() {
                let to = points[(at + 1) % points.len()];
                if *from != to {
                    edges.push([
                        from.x - origin.x,
                        from.y - origin.y,
                        to.x - origin.x,
                        to.y - origin.y,
                    ]);
                }
            }
            if index == 0 {
                contour = edges.len();
            }
        }
        Ok(Self {
            origin,
            edges,
            contour,
        })
    }

    fn ends(&self, edge: usize) -> (Vec2, Vec2) {
        let [ax, ay, bx, by] = self.edges[edge];
        ((ax as f64, ay as f64), (bx as f64, by as f64))
    }

    /// Whether `point` lies in the region, by counting the edges a ray
    /// from it crosses.
    fn contains(&self, point: Vec2) -> bool {
        let mut inside = false;
        for edge in 0..self.edges.len() {
            let (a, b) = self.ends(edge);
            if (a.1 > point.1) != (b.1 > point.1)
                && point.0 < a.0 + (point.1 - a.1) / (b.1 - a.1) * (b.0 - a.0)
            {
                inside = !inside;
            }
        }
        inside
    }

    fn on_boundary(&self, point: Vec2) -> bool {
        (0..self.edges.len()).any(|edge| {
            let (a, b) = self.ends(edge);
            distance_to_segment(point, a, b) < EPSILON
        })
    }

    fn point(&self, at: Vec2) -> Point2 {
        Point2 {
            x: at.0.round() as i64 + self.origin.x,
            y: at.1.round() as i64 + self.origin.y,
        }
    }

    fn local(&self, point: Point2) -> Vec2 {
        (
            (point.x - self.origin.x) as f64,
            (point.y - self.origin.y) as f64,
        )
    }
}

/// The diagram's edges that make up the centre lines, and the walk along
/// them (`MedialAxis`).
struct Skeleton<'a> {
    outline: &'a Outline,
    diagram: Diagram,
    /// For every pair of half-edges, by the lower of their two numbers:
    /// whether it is part of a centre line not yet walked, and the
    /// region's width at its start and end.
    active: Vec<bool>,
    widths: Vec<(f64, f64)>,
    min_width: f64,
    max_width: f64,
}

impl Skeleton<'_> {
    fn twin(&self, edge: EdgeIndex) -> Result<EdgeIndex, BvError> {
        self.diagram.edge_get_twin(edge)
    }

    /// The pair an edge belongs to, and whether the edge is the second of
    /// the pair, which runs the other way.
    fn pair(&self, edge: EdgeIndex) -> Result<(usize, bool), BvError> {
        let twin = self.twin(edge)?;
        Ok((edge.usize().min(twin.usize()), edge.usize() > twin.usize()))
    }

    fn ends(&self, edge: EdgeIndex) -> Result<Option<(Vec2, Vec2)>, BvError> {
        let from = self.diagram.edge_get_vertex0(edge)?;
        let to = self.diagram.edge_get_vertex1(edge)?;
        let (Some(from), Some(to)) = (from, to) else {
            return Ok(None);
        };
        let (from, to) = (self.diagram.vertex(from)?, self.diagram.vertex(to)?);
        Ok(Some(((from.x(), from.y()), (to.x(), to.y()))))
    }

    /// Whether an edge runs along the middle of a stretch of the region
    /// that is within the widths asked for, and if so how wide the region
    /// is at its two ends (`MedialAxis::validate_edge`).
    fn measure(&self, edge: EdgeIndex) -> Result<Option<(f64, f64)>, BvError> {
        let Some((from, to)) = self.ends(edge)? else {
            return Ok(None);
        };
        let left = self.diagram.cell(self.diagram.edge(edge)?.cell()?)?;
        let right = self
            .diagram
            .cell(self.diagram.edge(self.twin(edge)?)?.cell()?)?;
        let side = |cell: &boostvoronoi::prelude::Cell, at: Vec2| {
            let (a, b) = self.outline.ends(cell.source_index().usize());
            match cell.source_category() {
                SourceCategory::Segment => 2.0 * distance_to_segment(at, a, b),
                SourceCategory::SegmentStart | SourceCategory::SinglePoint => 2.0 * distance(at, a),
                SourceCategory::SegmentEnd => 2.0 * distance(at, b),
            }
        };
        let (w0, w1) = (side(right, from), side(left, to));
        if left.contains_segment() && right.contains_segment() {
            // Only the middle of two sides that face each other counts:
            // the region is long and thin there.
            let heading = |cell: &boostvoronoi::prelude::Cell| {
                let (a, b) = self.outline.ends(cell.source_index().usize());
                let angle = (b.1 - a.1).atan2(b.0 - a.0);
                if angle < 0.0 {
                    angle + std::f64::consts::TAU
                } else {
                    angle
                }
            };
            let mut angle = (heading(right) - heading(left)).abs();
            if angle > std::f64::consts::PI {
                angle = std::f64::consts::TAU - angle;
            }
            if std::f64::consts::PI - angle > std::f64::consts::PI / 8.0
                && (w0 < EPSILON || w1 < EPSILON || distance(from, to) >= self.min_width)
            {
                return Ok(None);
            }
        } else if w0 < EPSILON || w1 < EPSILON {
            return Ok(None);
        }
        let wanted = (w0 >= self.min_width || w1 >= self.min_width)
            && (w0 <= self.max_width || w1 <= self.max_width);
        Ok(wanted.then_some((w0, w1)))
    }

    /// Follows the centre line on from the end of `edge` for as long as
    /// exactly one unwalked edge continues it
    /// (`MedialAxis::process_edge_neighbors`).
    fn follow(&mut self, mut edge: EdgeIndex, line: &mut ThickPolyline) -> Result<(), BvError> {
        loop {
            let twin = self.twin(edge)?;
            let mut count = 0;
            let mut first = None;
            let mut neighbour = self.diagram.edge_rot_next(twin);
            while let Some(candidate) = neighbour.filter(|candidate| *candidate != twin) {
                if self.active[self.pair(candidate)?.0] {
                    first.get_or_insert(candidate);
                    count += 1;
                }
                neighbour = self.diagram.edge_rot_next(candidate);
            }
            match (count, first) {
                (1, Some(next)) => {
                    let (pair, reversed) = self.pair(next)?;
                    self.active[pair] = false;
                    let Some((_, to)) = self.ends(next)? else {
                        return Ok(());
                    };
                    line.points.push(self.outline.point(to));
                    let (start, end) = self.widths[pair];
                    if reversed {
                        line.widths.extend([end, start]);
                    } else {
                        line.widths.extend([start, end]);
                    }
                    edge = next;
                }
                (0, _) => {
                    line.open_ends.1 = true;
                    return Ok(());
                }
                // Three or more lines meet here.
                _ => return Ok(()),
            }
        }
    }

    /// Every centre line, each walked both ways from the edge it was
    /// found by (`MedialAxis::build`).
    fn lines(&mut self) -> Result<Vec<ThickPolyline>, BvError> {
        let count = self.diagram.edges().len();
        // The edges worth keeping: inside the region and along its middle.
        for index in 0..count {
            let edge = self.diagram.edge_index_unchecked(index);
            let (pair, reversed) = self.pair(edge)?;
            if reversed || !self.diagram.edge(edge)?.is_primary() {
                continue;
            }
            let Some((from, to)) = self.ends(edge)? else {
                continue;
            };
            let inside = |at: Vec2| !self.outline.on_boundary(at) && self.outline.contains(at);
            if !inside(from) && !inside(to) {
                continue;
            }
            if let Some(widths) = self.measure(edge)? {
                self.active[pair] = true;
                self.widths[pair] = widths;
            }
        }
        let mut lines = Vec::new();
        for index in 0..count {
            let seed = self.diagram.edge_index_unchecked(index);
            let (pair, reversed) = self.pair(seed)?;
            if reversed || !self.active[pair] {
                continue;
            }
            self.active[pair] = false;
            let Some((from, to)) = self.ends(seed)? else {
                continue;
            };
            let (start, end) = self.widths[pair];
            let mut line = ThickPolyline {
                points: vec![self.outline.point(from), self.outline.point(to)],
                widths: vec![start, end],
                open_ends: (false, false),
            };
            self.follow(seed, &mut line)?;
            let mut back = ThickPolyline::default();
            self.follow(self.twin(seed)?, &mut back)?;
            back.points.reverse();
            back.widths.reverse();
            back.points.append(&mut line.points);
            back.widths.append(&mut line.widths);
            line.points = back.points;
            line.widths = back.widths;
            line.open_ends.0 = back.open_ends.1;
            if line.points.first() == line.points.last() {
                line.open_ends = (false, false);
            }
            lines.push(line);
        }
        Ok(lines)
    }
}

/// The centre lines of the stretches of `region` between `min_width` and
/// `max_width` across, both in the units of its points. Free ends are run
/// out to the region's edge, and stubs shorter than twice the widest
/// stretch are dropped (`ExPolygon::medial_axis`).
pub fn medial_axis(
    region: &ExPolygon,
    min_width: f64,
    max_width: f64,
) -> Result<Vec<ThickPolyline>, MedialError> {
    let outline = Outline::new(region)?;
    if outline.edges.len() < 3 {
        return Ok(Vec::new());
    }
    let diagram = Builder::<i64>::default()
        .with_segments(outline.edges.iter())?
        .build()?;
    let pairs = diagram.edges().len();
    let mut skeleton = Skeleton {
        outline: &outline,
        diagram,
        active: vec![false; pairs],
        widths: vec![(0.0, 0.0); pairs],
        min_width,
        max_width,
    };
    let mut lines = skeleton.lines()?;

    let widest = lines
        .iter()
        .flat_map(|line| &line.widths)
        .copied()
        .fold(0.0, f64::max);
    // Where the line through `from` and `to`, carried on past `to`, leaves
    // the contour. Of several crossings the reference takes whichever
    // edge comes first; the one nearest `to` is taken here.
    let run_out = |from: Vec2, to: Vec2| -> Option<Point2> {
        let length = distance(from, to);
        if length == 0.0 {
            return None;
        }
        let beyond = (
            to.0 + (to.0 - from.0) / length * max_width,
            to.1 + (to.1 - from.1) / length * max_width,
        );
        (0..outline.contour)
            .filter_map(|edge| crossing((from, beyond), outline.ends(edge)))
            .min_by(|a, b| distance(*a, to).total_cmp(&distance(*b, to)))
            .map(|at| outline.point(at))
    };
    let mut removed = false;
    lines.retain_mut(|line| {
        let count = line.points.len();
        let front = outline.local(line.points[0]);
        let back = outline.local(line.points[count - 1]);
        let (mut second, mut before_last) = (
            outline.local(line.points[1]),
            outline.local(line.points[count - 2]),
        );
        // A line of one segment is run out from its middle, so neither
        // end finds the crossing meant for the other.
        if count == 2 {
            let middle = ((front.0 + back.0) / 2.0, (front.1 + back.1) / 2.0);
            (second, before_last) = (middle, middle);
        }
        let new_front = (line.open_ends.0 && !outline.on_boundary(front))
            .then(|| run_out(second, front))
            .flatten();
        let new_back = (line.open_ends.1 && !outline.on_boundary(back))
            .then(|| run_out(before_last, back))
            .flatten();
        if let Some(point) = new_front {
            line.points[0] = point;
        }
        if let Some(point) = new_back {
            line.points[count - 1] = point;
        }
        let stub = (line.open_ends.0 || line.open_ends.1) && line.length() < widest * 2.0;
        removed |= stub;
        !stub
    });

    // With stubs gone, lines that met at one are joined end to end.
    if removed {
        let mut index = 0;
        while index < lines.len() {
            let mut other = index + 1;
            while other < lines.len() {
                if lines[index].open_ends.0 && lines[index].open_ends.1 {
                    break;
                }
                let (first, last) = (lines[index].points[0], *lines[index].points.last().unwrap());
                let (other_first, other_last) =
                    (lines[other].points[0], *lines[other].points.last().unwrap());
                if last == other_last {
                    lines[other].reverse();
                } else if first == other_last {
                    lines[index].reverse();
                    lines[other].reverse();
                } else if first == other_first {
                    lines[index].reverse();
                } else if last != other_first {
                    other += 1;
                    continue;
                }
                let joined = lines.remove(other);
                let line = &mut lines[index];
                line.points.extend(&joined.points[1..]);
                line.widths.extend(&joined.widths);
                line.open_ends.1 = joined.open_ends.1;
                other = index + 1;
            }
            index += 1;
        }
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use edge_geom::{Polygon, scaled, unscaled};

    use super::*;

    fn region(points: &[(f64, f64)]) -> ExPolygon {
        ExPolygon {
            contour: Polygon {
                points: points
                    .iter()
                    .map(|(x, y)| Point2::from_mm(*x, *y))
                    .collect(),
            },
            holes: Vec::new(),
        }
    }

    #[test]
    fn a_strip_has_one_line_down_its_middle() {
        // 10 mm long and 0.3 mm across.
        let strip = region(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.3), (0.0, 0.3)]);
        let lines = medial_axis(&strip, scaled(0.05) as f64, scaled(0.8) as f64).unwrap();
        let [line] = lines.as_slice() else {
            panic!("expected one line, got {}", lines.len())
        };
        // It runs the whole length, at mid height, 0.3 mm wide between
        // the ends.
        assert!((unscaled(line.length() as i64) - 10.0).abs() < 1e-3);
        assert!(
            line.points
                .iter()
                .all(|point| (unscaled(point.y) - 0.15).abs() < 1e-4)
        );
        let middle = &line.widths[1..line.widths.len() - 1];
        assert!(
            middle
                .iter()
                .all(|width| (unscaled(*width as i64) - 0.3).abs() < 1e-4)
        );
    }

    #[test]
    fn a_wide_region_has_no_line() {
        let block = region(&[(0.0, 0.0), (10.0, 0.0), (10.0, 5.0), (0.0, 5.0)]);
        assert!(
            medial_axis(&block, scaled(0.05) as f64, scaled(0.8) as f64)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_wedge_is_one_line_that_widens_from_its_tip() {
        // 10 mm long, tapering from 0.8 mm to a point. An edge of the
        // diagram is kept whole when either end is within the widths, so
        // the line runs from the tip to the wide end.
        let wedge = region(&[(0.0, 0.0), (10.0, 0.4), (0.0, 0.8)]);
        let lines = medial_axis(&wedge, scaled(0.05) as f64, scaled(0.8) as f64).unwrap();
        let [line] = lines.as_slice() else {
            panic!("expected one line, got {}", lines.len())
        };
        let length = unscaled(line.length() as i64);
        assert!((length - 10.0).abs() < 0.05, "{length} mm");
        let (thin, wide) = line
            .widths
            .iter()
            .fold((f64::MAX, 0.0_f64), |(low, high), width| {
                (low.min(*width), high.max(*width))
            });
        assert!(unscaled(thin as i64) < 0.01);
        assert!((unscaled(wide as i64) - 0.77).abs() < 0.03, "{wide}");
    }

    #[test]
    fn a_ring_gives_a_closed_line() {
        let outer: Vec<(f64, f64)> = (0..48)
            .map(|step| {
                let angle = f64::from(step) / 48.0 * std::f64::consts::TAU;
                (5.0 * angle.cos(), 5.0 * angle.sin())
            })
            .collect();
        let mut ring = region(&outer);
        ring.holes.push(Polygon {
            points: outer
                .iter()
                .rev()
                .map(|(x, y)| Point2::from_mm(x * 0.94, y * 0.94))
                .collect(),
        });
        let lines = medial_axis(&ring, scaled(0.05) as f64, scaled(0.8) as f64).unwrap();
        let [line] = lines.as_slice() else {
            panic!("expected one line, got {}", lines.len())
        };
        assert!(line.is_closed());
        // Round the middle of the ring: radius 4.85 mm.
        let length = unscaled(line.length() as i64);
        assert!(
            (length - std::f64::consts::TAU * 4.85).abs() < 0.2,
            "{length} mm"
        );
    }
}
