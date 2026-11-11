//! The way round: where a straight move crosses the rings it is routed
//! by, it follows each ring from where it first meets it to where it
//! last leaves it, the shorter way about, and is then straightened as
//! far as the rings allow.
//!
//! Ported from the reference's AvoidCrossingPerimeters.cpp:
//! `avoid_perimeters_inner`, `extend_for_closest_lines`,
//! `get_closest_lines_in_radius`, `get_shortest_direction`,
//! `simplify_travel` and the helpers that step a point off a ring.

use edge_geom::{Coord, Point2, Polygon, scaled};

use crate::{
    grid::Grid,
    shape::{EPSILON, lengths_along},
};

type V = (f64, f64);

fn between(from: Point2, to: Point2) -> V {
    ((to.x - from.x) as f64, (to.y - from.y) as f64)
}

fn norm(v: V) -> f64 {
    v.0.hypot(v.1)
}

/// As Eigen's `normalized`: a vector of no length stays one.
fn normalized(v: V) -> V {
    let length = norm(v);
    if length > 0.0 {
        (v.0 / length, v.1 / length)
    } else {
        v
    }
}

fn distance(a: Point2, b: Point2) -> f64 {
    norm(between(a, b))
}

/// Rings a move is routed by, with what is needed to find where a move
/// meets them (`AvoidCrossingPerimeters::Boundary`).
pub(crate) struct Boundary {
    pub(crate) rings: Vec<Polygon>,
    /// How far round each ring every point of it is, and last the
    /// ring's whole length.
    along: Vec<Vec<f64>>,
    grid: Grid,
}

/// Where a move meets a ring.
#[derive(Clone, Copy, Debug)]
struct Meeting {
    ring: usize,
    /// The edge of the ring it is on, by the edge's first point.
    edge: usize,
    point: Point2,
    /// How far round the ring from its first point.
    along: f64,
    /// Kept when the way is straightened.
    kept: bool,
}

/// A point of the way round.
#[derive(Clone, Copy, Debug)]
struct Step {
    point: Point2,
    kept: bool,
}

impl Boundary {
    pub(crate) fn new(mut rings: Vec<Polygon>) -> Self {
        rings.retain(|ring| ring.points.len() >= 3);
        Boundary {
            along: rings
                .iter()
                .map(|ring| lengths_along(&ring.points))
                .collect(),
            // The reference's cells are a millimetre too.
            grid: Grid::new(&rings, scaled(1.0)),
            rings,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rings.is_empty()
    }

    fn edge(&self, ring: usize, edge: usize) -> (Point2, Point2) {
        let points = &self.rings[ring].points;
        (points[edge], points[(edge + 1) % points.len()])
    }

    fn meeting(&self, ring: usize, edge: usize, point: Point2, kept: bool) -> Meeting {
        Meeting {
            ring,
            edge,
            point,
            along: self.along[ring][edge] + distance(self.rings[ring].points[edge], point),
            kept,
        }
    }

    /// Everywhere the segment from `a` to `b` meets a ring, an end on a
    /// ring counting (`AllIntersectionsVisitor`, `line_alg::intersection`).
    fn meetings(&self, a: Point2, b: Point2) -> Vec<Meeting> {
        let v1 = between(a, b);
        let mut found = Vec::new();
        for (ring, edge) in self.grid.along(a, b) {
            let (c, d) = self.edge(ring, edge);
            let v2 = between(c, d);
            let denom = v1.0 * v2.1 - v1.1 * v2.0;
            // Parallel lines meet nowhere, or all along.
            if denom.abs() < 1e-4 {
                continue;
            }
            let v12 = between(c, a);
            let t1 = (v2.0 * v12.1 - v2.1 * v12.0) / denom;
            let t2 = (v1.0 * v12.1 - v1.1 * v12.0) / denom;
            if (0.0..=1.0).contains(&t1) && (0.0..=1.0).contains(&t2) {
                let point = Point2 {
                    x: (a.x as f64 + t1 * v1.0) as Coord,
                    y: (a.y as f64 + t1 * v1.1) as Coord,
                };
                found.push(self.meeting(ring, edge, point, false));
            }
        }
        found
    }

    /// Whether the segment from `a` to `b` meets or touches any ring
    /// (`FirstIntersectionVisitor`, `Geometry::segments_intersect`).
    pub(crate) fn blocks(&self, a: Point2, b: Point2) -> bool {
        self.grid.any_along(a, b, |ring, edge| {
            let (c, d) = self.edge(ring, edge);
            segments_intersect(c, d, a, b)
        })
    }

    /// Whether `point` is within the rings, counting how many of them a
    /// ray from it crosses: inside an outline and not in a hole of it.
    pub(crate) fn holds(&self, point: Point2) -> bool {
        // Where an edge crosses the level of the point, which says in
        // which one cell of those it lies in it is to be counted.
        let crossing = |ring: usize, edge: usize| {
            let (c, d) = self.edge(ring, edge);
            if c.y == d.y {
                return c.x;
            }
            let t = (point.y - c.y) as f64 / (d.y - c.y) as f64;
            (c.x as f64 + t.clamp(0.0, 1.0) * (d.x - c.x) as f64) as Coord
        };
        let mut inside = false;
        self.grid
            .each_rightward(point, crossing, |ring, edge, counted| {
                let (c, d) = self.edge(ring, edge);
                if counted && (c.y > point.y) != (d.y > point.y) {
                    let side = i128::from(d.x - c.x) * i128::from(point.y - c.y)
                        - i128::from(point.x - c.x) * i128::from(d.y - c.y);
                    if (side > 0) == (d.y > c.y) {
                        inside = !inside;
                    }
                }
            });
        inside
    }

    /// The edges within `radius` of `center`, each with its nearest
    /// point, nearest first (`get_closest_lines_in_radius`). Of two as
    /// near as each other the one that comes first in its ring comes
    /// first; the reference leaves that to its sort.
    fn closest(&self, center: Point2, radius: f64) -> Vec<Meeting> {
        let mut found: Vec<(f64, Meeting)> = Vec::new();
        for (ring, edge) in self.grid.near(center, radius as Coord) {
            let (a, b) = self.edge(ring, edge);
            let v = between(a, b);
            let va = between(a, center);
            let l2 = v.0 * v.0 + v.1 * v.1;
            let t = if l2 == 0.0 {
                0.0
            } else {
                (va.0 * v.0 + va.1 * v.1) / l2
            };
            let nearest = if t <= 0.0 {
                a
            } else if t >= 1.0 {
                b
            } else {
                Point2 {
                    x: (a.x as f64 + t * v.0) as Coord,
                    y: (a.y as f64 + t * v.1) as Coord,
                }
            };
            let squared = {
                let to = between(center, nearest);
                to.0 * to.0 + to.1 * to.1
            };
            if squared <= radius * radius {
                found.push((squared, self.meeting(ring, edge, nearest, true)));
            }
        }
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found.into_iter().map(|(_, meeting)| meeting).collect()
    }

    /// When the rings are drawn well in from the outline a move that
    /// starts or ends near one need not cross it. This adds a meeting
    /// with the ring nearest each end, so the move still goes along it
    /// (`extend_for_closest_lines`).
    fn extended(
        &self,
        meetings: Vec<Meeting>,
        start: Point2,
        end: Point2,
        radius: f64,
    ) -> Vec<Meeting> {
        let start_lines = self.closest(start, radius);
        let end_lines = self.closest(end, radius);
        // Both ends near one ring: the whole way can go along it, and
        // every other meeting is dropped.
        for near_end in &end_lines {
            if let Some(near_start) = start_lines.iter().find(|near| near.ring == near_end.ring) {
                return vec![*near_start, *near_end];
            }
        }
        // An edge near `close_to`, of the same ring as `meeting` and
        // nearer `close_to` than it.
        let closer = |lines: &[Meeting], meeting: &Meeting, close_to: Point2| -> Option<Meeting> {
            let old = {
                let to = between(close_to, meeting.point);
                to.0 * to.0 + to.1 * to.1
            };
            lines.iter().copied().find(|near| {
                let new = between(close_to, near.point);
                near.ring == meeting.ring
                    && old <= radius * radius
                    && new.0 * new.0 + new.1 * new.1 < old
            })
        };
        // The nearest edge of a ring the move also meets, looking from
        // the far end of the move: going along a ring it meets anyway
        // keeps the way round short.
        fn of_a_ring_met<'a>(
            lines: &[Meeting],
            mut met: impl Iterator<Item = &'a Meeting>,
        ) -> Option<Meeting> {
            met.find(|meeting| lines.iter().any(|near| near.ring == meeting.ring))
                .and_then(|meeting| lines.iter().copied().find(|near| near.ring == meeting.ring))
        }
        let mut meetings = meetings;
        if !meetings.is_empty() && !start_lines.is_empty() {
            match closer(&start_lines, &meetings[0], start) {
                Some(near) => meetings[0] = near,
                None => {
                    let near = of_a_ring_met(&start_lines, meetings.iter().rev())
                        .unwrap_or(start_lines[0]);
                    meetings.insert(0, near);
                }
            }
        }
        if !meetings.is_empty() && !end_lines.is_empty() {
            let last = meetings.len() - 1;
            match closer(&end_lines, &meetings[last], end) {
                Some(near) => meetings[last] = near,
                None => {
                    let near = of_a_ring_met(&end_lines, meetings.iter()).unwrap_or(end_lines[0]);
                    meetings.push(near);
                }
            }
        }
        meetings
    }

    /// The first point of a ring that is not at `point`, looking back
    /// from `from` or on from it.
    ///
    /// Not as the reference, on purpose: its
    /// `find_first_different_vertex<false>` takes its first step forward
    /// whichever way it is looking, so for a point at the start of an
    /// edge it finds the edge's far end on both sides, the two normals
    /// cancel, and the point is left on the ring instead of stepped off
    /// it.
    fn other_point(&self, ring: usize, from: usize, point: Point2, back: bool) -> Point2 {
        let points = &self.rings[ring].points;
        let count = points.len();
        let mut index = from;
        for _ in 0..count {
            if points[index] != point {
                break;
            }
            index = if back {
                (index + count - 1) % count
            } else {
                (index + 1) % count
            };
        }
        points[index]
    }

    /// `point`, which lies on edge `edge` of a ring, stepped a hair to
    /// the ring's left: into what the ring bounds, off the ring itself
    /// (`get_middle_point_offset`).
    fn stepped_in(&self, ring: usize, edge: usize, point: Point2) -> Point2 {
        let count = self.rings[ring].points.len();
        let left = self.other_point(ring, edge, point, true);
        let right = self.other_point(ring, (edge + 1) % count, point, false);
        let perp = |v: V| (-v.1, v.0);
        let (a, b) = (
            normalized(perp(between(left, point))),
            normalized(perp(between(point, right))),
        );
        let normal = normalized((a.0 + b.0, a.1 + b.1));
        Point2 {
            x: point.x + (normal.0 * EPSILON as f64) as Coord,
            y: point.y + (normal.1 * EPSILON as f64) as Coord,
        }
    }

    /// Corner `index` of a ring stepped a hair to the ring's left
    /// (`get_polygon_vertex_offset`).
    fn corner_stepped_in(&self, ring: usize, index: usize) -> Point2 {
        let count = self.rings[ring].points.len();
        self.stepped_in(
            ring,
            (index + count - 1) % count,
            self.rings[ring].points[index],
        )
    }

    /// Whether the shorter way round a ring from `first` to `second` is
    /// the way the ring runs.
    ///
    /// Not as the reference, on purpose: `get_shortest_direction` takes
    /// from each way's length what lies between a meeting and the ends
    /// of the edge it is on, a correction left from when a meeting was
    /// placed at its edge's start; a meeting is now placed where it is,
    /// so the lengths are already right and the correction sends some
    /// moves the longer way about, by up to the length of the two edges.
    fn forward_is_shorter(&self, first: &Meeting, second: &Meeting) -> bool {
        let whole = self.along[first.ring][self.rings[first.ring].points.len()];
        let forward = (second.along - first.along).rem_euclid(whole);
        forward < whole - forward
    }

    /// The way from `start` to `end` along the rings, and how many times
    /// the move met them (`avoid_perimeters_inner`). `spacing` is the
    /// spacing of the walls, in scaled units.
    pub(crate) fn way(&self, start: Point2, end: Point2, spacing: f64) -> (Vec<Point2>, usize) {
        let (mut start, mut end) = (start, end);
        let mut meetings = self.meetings(start, end);
        let mut dir = between(start, end);
        // Rings drawn well in may not be crossed at all: look for one
        // near either end, and cross between the nearest points.
        if meetings.is_empty() {
            let radius = 1.5 * spacing;
            let near_start = self.closest(start, radius);
            let near_end = self.closest(end, radius);
            if !(near_start.is_empty() && near_end.is_empty()) {
                let new_start = near_start.first().map_or(start, |near| near.point);
                let new_end = near_end.first().map_or(end, |near| near.point);
                let new_dir = between(new_start, new_end);
                // Each a hair further out along the line, to cross for sure.
                let unit = normalized(new_dir);
                let hair = (
                    (unit.0 * EPSILON as f64) as Coord,
                    (unit.1 * EPSILON as f64) as Coord,
                );
                let new_start = Point2 {
                    x: new_start.x - hair.0,
                    y: new_start.y - hair.1,
                };
                let new_end = Point2 {
                    x: new_end.x + hair.0,
                    y: new_end.y + hair.1,
                };
                dir = new_dir;
                meetings = self.meetings(new_start, new_end);
                if !meetings.is_empty() {
                    start = new_start;
                    end = new_end;
                }
            }
        }
        // In the order the move comes to them. Meetings at one place
        // keep the order of their rings and edges; the reference leaves
        // that to its sort.
        let reached =
            |meeting: &Meeting| meeting.point.x as f64 * dir.0 + meeting.point.y as f64 * dir.1;
        meetings.sort_by(|a, b| reached(a).total_cmp(&reached(b)));
        // "Search radius should always be at least equals to the value
        // of offset used for computing boundaries."
        let meetings = self.extended(meetings, start, end, 2.0 * spacing);

        let mut way = vec![Step {
            point: start,
            kept: false,
        }];
        let mut index = 0;
        while index < meetings.len() {
            let first = meetings[index];
            let count = self.rings[first.ring].points.len();
            way.push(Step {
                point: self.stepped_in(first.ring, first.edge, first.point),
                kept: first.kept,
            });
            // The last place the move meets the same ring.
            let last = (index + 1..meetings.len())
                .rev()
                .find(|other| meetings[*other].ring == first.ring);
            if let Some(last) = last {
                let second = meetings[last];
                // Round the ring between the two, corner by corner.
                let mut edge = first.edge;
                if self.forward_is_shorter(&first, &second) {
                    while edge != second.edge {
                        edge = (edge + 1) % count;
                        way.push(Step {
                            point: self.corner_stepped_in(first.ring, edge),
                            kept: false,
                        });
                    }
                } else {
                    while edge != second.edge {
                        way.push(Step {
                            point: self.corner_stepped_in(first.ring, edge),
                            kept: false,
                        });
                        edge = (edge + count - 1) % count;
                    }
                }
                way.push(Step {
                    point: self.stepped_in(second.ring, second.edge, second.point),
                    kept: second.kept,
                });
                // Whatever is met in between is passed by.
                index = last;
            }
            index += 1;
        }
        way.push(Step {
            point: end,
            kept: false,
        });
        if !meetings.is_empty() {
            way = self.straightened(&way);
        }
        (
            way.into_iter().map(|step| step.point).collect(),
            meetings.len(),
        )
    }

    /// The way with every point left out that it can go straight past
    /// without meeting a ring (`simplify_travel`): from each point it
    /// goes on to the last of those ahead that it can reach straight,
    /// looking no further than the first that is kept, which is never
    /// gone straight to. The reference tries every point ahead in turn
    /// and keeps the last that can be reached; looking from the far end
    /// back finds the same one, and sooner where the way is open.
    fn straightened(&self, way: &[Step]) -> Vec<Step> {
        let mut out = Vec::with_capacity(way.len());
        out.push(way[0]);
        let mut index = 1;
        while index < way.len() {
            let current = way[index - 1].point;
            if !way[index].kept {
                let ahead = index + 1;
                let kept = way[ahead..].iter().position(|step| step.kept);
                let limit = kept.map_or(way.len(), |kept| ahead + kept);
                let reached = (ahead..limit).rev().find(|&ahead| {
                    way[ahead].point == current || !self.blocks(current, way[ahead].point)
                });
                index = reached.unwrap_or(index);
            }
            out.push(way[index]);
            index += 1;
        }
        out
    }
}

/// Whether segments `ip1`-`ip2` and `jp1`-`jp2` meet, touching counted
/// (`Geometry::segments_intersect`).
fn segments_intersect(ip1: Point2, ip2: Point2, jp1: Point2, jp2: Point2) -> bool {
    let signs = |a: Point2, b: Point2, c: Point2, d: Point2| {
        let cross = |p: Point2| {
            (i128::from(b.x - a.x) * i128::from(p.y - a.y)
                - i128::from(b.y - a.y) * i128::from(p.x - a.x))
            .signum()
        };
        (cross(c), cross(d))
    };
    let sign1 = signs(ip1, ip2, jp1, jp2);
    let sign2 = signs(jp1, jp2, ip1, ip2);
    let (test1, test2) = (sign1.0 * sign1.1, sign2.0 * sign2.1);
    if test1 > 0 || test2 > 0 {
        return false;
    }
    if test1 != 0 || test2 != 0 {
        return true;
    }
    if sign1 != (0, 0) {
        // Not in one line, and an end of each on the line of the other:
        // they share that end and no more, which the reference does not
        // count.
        return false;
    }
    // In one line: whether they overlap along it.
    let along_x = (ip2.x - ip1.x).abs() > (ip2.y - ip1.y).abs();
    let axis = |p: Point2| if along_x { p.x } else { p.y };
    let (i, j) = (axis(ip1).min(axis(ip2)), axis(ip1).max(axis(ip2)));
    let (k, l) = (axis(jp1).min(axis(jp2)), axis(jp1).max(axis(jp2)));
    i <= l && k <= j
}

/// The length of a way in scaled units.
pub(crate) fn length(way: &[Point2]) -> f64 {
    way.windows(2).map(|pair| distance(pair[0], pair[1])).sum()
}
