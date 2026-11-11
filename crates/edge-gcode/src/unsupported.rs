//! Wall loops with nothing under them (`unsupported_wall_last`), after
//! PerimeterGenerator.cpp `is_unsupported_loop` and
//! `defer_unsupported_loops`, and GCode.cpp `extrude_perimeters`.
//!
//! A loop wholly over air has nothing to lean on, so it is laid after the
//! island's other walls, innermost first, whatever the wall order. One
//! that runs within a wall spacing and a half of a loop that is anchored
//! (or of one anchored through another) keeps its place ahead of the
//! fill; one that touches none of them has only the fill to rest on, and
//! waits until the island's fill is down.
//!
//! The reference tells such a loop when it makes the walls, by every one
//! of its paths being an overhang wall; here walls are cut into what
//! rests on the layer beneath and what hangs clear as they are laid
//! (`overhang.rs`), and a loop is over air when that cut leaves it one
//! stretch, clear all the way round.

use edge_extrusion::{ExtrusionLoop, ExtrusionPath, Role};
use edge_geom::{Point2, unscaled};

use crate::overhang::{self, Below};

/// What becomes of an island's loops, each by its place among them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Held {
    /// Wholly over air: laid after the island's other walls.
    pub(crate) over_air: Vec<bool>,
    /// Over air and out of reach of every anchored wall: laid once the
    /// island's fill is down (the reference's `print_after_infill`).
    pub(crate) after_fill: Vec<bool>,
}

impl Held {
    pub(crate) fn any_after_fill(&self) -> bool {
        self.after_fill.iter().any(|held| *held)
    }
}

/// Whether a loop hangs clear of the layer beneath all the way round:
/// cut where it leaves the outline beneath grown by `by` mm and where it
/// comes back, it is one stretch, and that one is clear.
pub(crate) fn over_air(below: &Below, by: f64, wall: &ExtrusionLoop) -> bool {
    let points = &wall.polygon.points;
    if points.len() < 3 {
        return false;
    }
    let mut closed = Vec::with_capacity(points.len() + 1);
    closed.extend_from_slice(points);
    closed.push(points[0]);
    overhang::stretches(below, by, &closed)
        .iter()
        .all(|stretch| stretch.clear)
}

/// Which of an island's loops wait, given which are over air. `thin` are
/// the island's walls that are no loops (thin walls), anchored like the
/// loops that rest on something; `reach` is how near an anchored wall a
/// loop has to come to be anchored too, in mm.
pub(crate) fn held(
    own: &[ExtrusionLoop],
    over_air: Vec<bool>,
    thin: &[&ExtrusionPath],
    reach: f64,
) -> Held {
    let mut after_fill = over_air.clone();
    // The anchored walls, each as the segments it is made of.
    let closed = |wall: &ExtrusionLoop| -> Vec<Point2> {
        let points = &wall.polygon.points;
        let mut closed = points.clone();
        if let Some(&first) = points.first() {
            closed.push(first);
        }
        closed
    };
    let mut anchored: Vec<Vec<Point2>> = (own.iter().zip(&over_air))
        .filter(|(_, over_air)| !**over_air)
        .map(|(wall, _)| closed(wall))
        .chain(thin.iter().map(|path| path.points.clone()))
        .collect();
    // A loop leaning on a loop that is itself anchored is anchored as
    // well, so the anchored spread out until none is left touching what
    // was reached.
    while !anchored.is_empty() {
        let reached: Vec<Line> = anchored.iter().flat_map(|points| lines(points)).collect();
        anchored.clear();
        for (index, wall) in own.iter().enumerate() {
            if !after_fill[index] {
                continue;
            }
            let near = wall
                .polygon
                .points
                .iter()
                .chain(wall.polygon.points.first())
                .any(|&point| reached.iter().any(|line| line.within(point, reach)));
            if near {
                after_fill[index] = false;
                anchored.push(closed(wall));
            }
        }
    }
    Held {
        over_air,
        after_fill,
    }
}

/// A segment in mm, with its box grown by nothing.
struct Line {
    a: (f64, f64),
    b: (f64, f64),
}

impl Line {
    /// Whether `point` is nearer the segment than `reach` mm.
    fn within(&self, point: Point2, reach: f64) -> bool {
        let (x, y) = (unscaled(point.x), unscaled(point.y));
        let (low_x, high_x) = (self.a.0.min(self.b.0), self.a.0.max(self.b.0));
        let (low_y, high_y) = (self.a.1.min(self.b.1), self.a.1.max(self.b.1));
        if x < low_x - reach || x > high_x + reach || y < low_y - reach || y > high_y + reach {
            return false;
        }
        let (dx, dy) = (self.b.0 - self.a.0, self.b.1 - self.a.1);
        let length = dx * dx + dy * dy;
        let along = if length == 0.0 {
            0.0
        } else {
            (((x - self.a.0) * dx + (y - self.a.1) * dy) / length).clamp(0.0, 1.0)
        };
        let (nx, ny) = (self.a.0 + along * dx, self.a.1 + along * dy);
        (x - nx).hypot(y - ny) < reach
    }
}

fn lines(points: &[Point2]) -> impl Iterator<Item = Line> + '_ {
    points.windows(2).map(|pair| Line {
        a: (unscaled(pair[0].x), unscaled(pair[0].y)),
        b: (unscaled(pair[1].x), unscaled(pair[1].y)),
    })
}

/// How near an anchored wall a loop has to come to be anchored: a wall
/// spacing and a half, the wider of the outer and the inner walls'
/// (`touch_distance`). The reference takes both from the region's
/// flows; here from the island's loops, where an island with one kind
/// of wall only has that one.
pub(crate) fn reach(own: &[ExtrusionLoop]) -> f64 {
    1.5 * own
        .iter()
        .filter(|wall| matches!(wall.role, Role::ExternalPerimeter | Role::Perimeter))
        .map(|wall| wall.flow.spacing)
        .fold(0.0, f64::max)
}

/// An island's walls in the order they are laid, `order` being that of
/// the wall order, with those over air moved after the rest, the
/// innermost of them first; the rest keep their order (a stable
/// partition and a stable sort, as the reference has them).
pub(crate) fn reorder(own: &[ExtrusionLoop], order: Vec<usize>, held: &Held) -> Vec<usize> {
    let (mut anchored, mut over_air): (Vec<usize>, Vec<usize>) =
        order.into_iter().partition(|&wall| !held.over_air[wall]);
    over_air.sort_by_key(|&wall| std::cmp::Reverse(own[wall].depth));
    anchored.extend(over_air);
    anchored
}

#[cfg(test)]
mod tests {
    use super::*;
    use edge_extrusion::Flow;
    use edge_geom::{ExPolygon, Polygon, scaled};

    fn square(x: f64, y: f64, side: f64, depth: usize) -> ExtrusionLoop {
        let at = |dx: f64, dy: f64| Point2 {
            x: scaled(x + dx),
            y: scaled(y + dy),
        };
        ExtrusionLoop {
            polygon: Polygon {
                points: vec![at(0.0, 0.0), at(side, 0.0), at(side, side), at(0.0, side)],
            },
            role: if depth == 0 {
                Role::ExternalPerimeter
            } else {
                Role::Perimeter
            },
            is_hole: false,
            depth,
            flow: Flow::new(0.45, 0.2, 0.4).unwrap(),
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: false,
        }
    }

    #[test]
    fn a_loop_is_over_air_only_when_none_of_it_rests_on_anything() {
        let beneath = [ExPolygon {
            contour: square(0.0, 0.0, 10.0, 0).polygon,
            holes: Vec::new(),
        }];
        let below = Below::new(&beneath, 0.6).unwrap();
        // Inside the outline beneath, half out of it, and clear of it.
        assert!(!over_air(&below, 0.2, &square(2.0, 2.0, 6.0, 0)));
        assert!(!over_air(&below, 0.2, &square(5.0, 2.0, 10.0, 0)));
        assert!(over_air(&below, 0.2, &square(20.0, 2.0, 6.0, 0)));
    }

    #[test]
    fn loops_over_air_wait_for_the_fill_unless_an_anchored_wall_is_near() {
        // An anchored loop, one over air beside it, one over air beside
        // that, and one far off.
        let own = [
            square(0.0, 0.0, 10.0, 0),
            square(10.3, 0.0, 10.0, 1),
            square(20.6, 0.0, 10.0, 2),
            square(50.0, 0.0, 10.0, 1),
        ];
        let held = held(&own, vec![false, true, true, true], &[], reach(&own));
        assert_eq!(held.after_fill, vec![false, false, false, true]);
        assert!(held.any_after_fill());
        // Over air, innermost first, after the anchored loops.
        assert_eq!(reorder(&own, vec![0, 1, 2, 3], &held), vec![0, 2, 1, 3]);
        // With nothing anchored at all, every loop waits.
        let none = super::held(&own, vec![true; 4], &[], reach(&own));
        assert_eq!(none.after_fill, vec![true; 4]);
    }
}
