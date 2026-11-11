use edge_geom::{Polygon, unscaled};

use super::*;

fn ring(corners: &[(f64, f64)]) -> Polygon {
    Polygon {
        points: corners
            .iter()
            .map(|&(x, y)| Point2::from_mm(x, y))
            .collect(),
    }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
    ring(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn solid(contour: Polygon) -> ExPolygon {
    ExPolygon {
        contour,
        holes: Vec::new(),
    }
}

/// A 40 mm square with a 20 mm square hole in the middle of it.
fn frame() -> ExPolygon {
    let mut hole = rect(10.0, 10.0, 30.0, 30.0);
    hole.points.reverse();
    ExPolygon {
        contour: rect(0.0, 0.0, 40.0, 40.0),
        holes: vec![hole],
    }
}

/// A U: 40 mm wide and 30 mm tall, with a slot 20 mm wide cut 20 mm
/// down into it from the top.
fn u_shape() -> ExPolygon {
    solid(ring(&[
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 30.0),
        (30.0, 30.0),
        (30.0, 10.0),
        (10.0, 10.0),
        (10.0, 30.0),
        (0.0, 30.0),
    ]))
}

const SPACING: f64 = 0.4;
const OUTER: f64 = 0.42;

fn layer_of(outline: &[ExPolygon], tops: &[ExPolygon], needs: Needs) -> Layer {
    Layer::new(
        &Slices {
            outline,
            tops,
            spacing: SPACING,
            outer_width: OUTER,
        },
        needs,
    )
    .unwrap()
}

fn mm(x: f64, y: f64) -> Point2 {
    Point2::from_mm(x, y)
}

fn length_mm(from: Point2, way: &[Point2]) -> f64 {
    let mut at = from;
    let mut length = 0.0;
    for &point in way {
        length += unscaled(point.x - at.x).hypot(unscaled(point.y - at.y));
        at = point;
    }
    length
}

/// Whether segments `ab` and `cd` cross at a point within both.
fn crosses(a: Point2, b: Point2, c: Point2, d: Point2) -> bool {
    let side = |p: Point2, q: Point2, r: Point2| {
        (i128::from(q.x - p.x) * i128::from(r.y - p.y)
            - i128::from(q.y - p.y) * i128::from(r.x - p.x))
        .signum()
    };
    side(a, b, c) * side(a, b, d) < 0 && side(c, d, a) * side(c, d, b) < 0
}

/// How many times the way from `from` crosses the outline of `regions`.
fn crossings(regions: &[ExPolygon], from: Point2, way: &[Point2]) -> usize {
    let mut at = from;
    let mut count = 0;
    for &point in way {
        for ring in regions.iter().flat_map(ExPolygon::rings) {
            let points = &ring.points;
            for index in 0..points.len() {
                if crosses(at, point, points[index], points[(index + 1) % points.len()]) {
                    count += 1;
                }
            }
        }
        at = point;
    }
    count
}

#[test]
fn a_move_within_the_walls_goes_straight() {
    let layer = layer_of(&[frame()], &[], Needs::default());
    // Along one side of the frame, in the fill.
    let (from, to) = (mm(3.0, 3.0), mm(37.0, 5.0));
    assert_eq!(layer.route(from, to, Kind::Part, None), vec![to]);
    // And a move to where the nozzle already is.
    assert_eq!(layer.route(from, from, Kind::Part, None), vec![from]);
}

#[test]
fn a_move_across_a_hole_goes_round_it() {
    let outline = [frame()];
    let layer = layer_of(&outline, &[], Needs::default());
    // From the left side of the frame to the right, straight over the hole.
    let (from, to) = (mm(5.0, 18.0), mm(35.0, 18.0));
    assert_eq!(crossings(&outline, from, &[to]), 2);
    let way = layer.route(from, to, Kind::Part, None);
    assert!(way.len() > 2, "{way:?}");
    assert_eq!(*way.last().unwrap(), to);
    assert_eq!(crossings(&outline, from, &way), 0);
    // Under the hole, the shorter way about: 0.6 mm clear of it.
    assert!(way.iter().all(|point| unscaled(point.y) <= 18.0 + 1e-6));
    let lowest = way.iter().map(|p| unscaled(p.y)).fold(f64::MAX, f64::min);
    assert!((lowest - 9.4).abs() < 0.01, "{lowest}");
    let length = length_mm(from, &way);
    assert!(length > 30.0 && length < 30.0 + 2.0 * 9.0, "{length}");
    // The same move the other way round takes the same way back.
    let back = layer.route(to, from, Kind::Part, None);
    assert!((length_mm(to, &back) - length).abs() < 0.01);
    // Asked again, the same answer: nothing is kept between moves.
    assert_eq!(layer.route(from, to, Kind::Part, None), way);
}

#[test]
fn a_move_across_a_concave_part_stays_inside_it() {
    let outline = [u_shape()];
    let layer = layer_of(&outline, &[], Needs::default());
    // From the top of one arm to the top of the other, over the slot.
    let (from, to) = (mm(5.0, 28.0), mm(35.0, 28.0));
    assert_eq!(crossings(&outline, from, &[to]), 2);
    let way = layer.route(from, to, Kind::Part, None);
    assert_eq!(crossings(&outline, from, &way), 0);
    // Down one arm, under the slot and up the other.
    let lowest = way.iter().map(|p| unscaled(p.y)).fold(f64::MAX, f64::min);
    assert!((lowest - 9.4).abs() < 0.01, "{lowest}");
    let length = length_mm(from, &way);
    assert!(length > 55.0 && length < 62.0, "{length}");
}

#[test]
fn a_move_between_two_islands_follows_each_and_crosses_between() {
    // Two U shapes side by side, 10 mm apart.
    let mut second = u_shape();
    for point in &mut second.contour.points {
        point.x += scaled(50.0);
    }
    let outline = [u_shape(), second];
    let layer = layer_of(&outline, &[], Needs::default());
    // From the left arm of the first to the right arm of the second:
    // straight, it would cross both slots.
    let (from, to) = (mm(5.0, 28.0), mm(85.0, 28.0));
    assert_eq!(crossings(&outline, from, &[to]), 6);
    let way = layer.route(from, to, Kind::Part, None);
    // Out of the one and into the other, and no more.
    assert_eq!(crossings(&outline, from, &way), 2);
    // A move from a part to a skirt outside it goes round holes only,
    // and these parts have none.
    let outside = layer_of(
        &outline,
        &[],
        Needs {
            outside: true,
            ..Needs::default()
        },
    );
    assert_eq!(outside.route(from, to, Kind::Outside, None), vec![to]);
}

#[test]
fn a_way_round_longer_than_allowed_is_not_taken() {
    let outline = [u_shape()];
    let layer = layer_of(&outline, &[], Needs::default());
    let (from, to) = (mm(5.0, 28.0), mm(35.0, 28.0));
    let way = layer.route(from, to, Kind::Part, None);
    let extra = length_mm(from, &way) - 30.0;
    assert!(extra > 25.0, "{extra}");
    // A length: just short of the extra, and just over it.
    let short = Some(Detour::Length(extra - 0.1));
    assert_eq!(layer.route(from, to, Kind::Part, short), vec![to]);
    let enough = Some(Detour::Length(extra + 0.1));
    assert_eq!(layer.route(from, to, Kind::Part, enough), way);
    // A share of the straight move, which is 30 mm long.
    let percent = 100.0 * extra / 30.0;
    let short = Some(Detour::Percent(percent - 1.0));
    assert_eq!(layer.route(from, to, Kind::Part, short), vec![to]);
    let enough = Some(Detour::Percent(percent + 1.0));
    assert_eq!(layer.route(from, to, Kind::Part, enough), way);
}

#[test]
fn a_move_to_the_outer_wall_runs_along_inside_it() {
    let outline = [solid(rect(0.0, 0.0, 40.0, 40.0))];
    let layer = layer_of(&outline, &[], Needs::default());
    // From the middle of the outer wall on one side to the middle of it
    // on the next: the straight move stays inside, yet it is taken in
    // behind the wall at both ends and along it between them. The two
    // points where it joins the boundary are kept when the way is
    // straightened, and, as in the reference, nothing is straightened
    // on to a kept point: the corner between them stays.
    let (from, to) = (mm(0.21, 20.0), mm(20.0, 0.21));
    let way = layer.route(from, to, Kind::Part, None);
    assert_eq!(way.len(), 4, "{way:?}");
    for point in &way[..way.len() - 1] {
        let (x, y) = (unscaled(point.x), unscaled(point.y));
        assert!(x >= 0.6 - 1e-6 && y >= 0.6 - 1e-6, "{x} {y}");
    }
    assert!((length_mm(from, &way) - (2.0 * 0.39 + 2.0 * 19.4)).abs() < 0.01);
    // A short hop from the wall behind to the outer wall is taken along
    // the boundary too, which here is next to no way at all.
    let (from, to) = (mm(0.62, 20.0), mm(0.21, 20.4));
    let way = layer.route(from, to, Kind::Part, None);
    assert!(length_mm(from, &way) < 0.9, "{way:?}");
}

#[test]
fn top_surfaces_are_gone_round_but_not_between_supports() {
    let outline = [solid(rect(0.0, 0.0, 40.0, 40.0))];
    let top = [solid(rect(15.0, 5.0, 25.0, 35.0))];
    let needs = Needs {
        supports: true,
        ..Needs::default()
    };
    let layer = layer_of(&outline, &top, needs);
    // Across the top surface, wall to wall.
    let (from, to) = (mm(0.21, 20.0), mm(39.79, 20.0));
    let way = layer.route(from, to, Kind::Part, None);
    assert_eq!(crossings(&top, from, &way), 0, "{way:?}");
    assert!(length_mm(from, &way) > 39.58 + 10.0);
    // A move between supports is not kept off it.
    let under = layer.route(from, to, Kind::Support, None);
    assert_eq!(crossings(&top, from, &under), 2);
    // With no supports on the layer there is one boundary for both.
    let plain = layer_of(&outline, &top, Needs::default());
    assert_eq!(plain.route(from, to, Kind::Support, None), way);
}

#[test]
fn a_move_outside_the_part_goes_round_a_hole_in_its_wall() {
    let outline = [frame()];
    let needs = Needs {
        outside: true,
        ..Needs::default()
    };
    let layer = layer_of(&outline, &[], needs);
    // From a skirt on one side of the frame to the other, over the hole.
    let (from, to) = (mm(-5.0, 18.0), mm(45.0, 18.0));
    let way = layer.route(from, to, Kind::Outside, None);
    assert!(way.len() > 2, "{way:?}");
    // In and out of the part, but not through the hole: the way round
    // is 0.2 mm into the hole's wall.
    assert_eq!(crossings(&outline, from, &way), 2);
    let lowest = way.iter().map(|p| unscaled(p.y)).fold(f64::MAX, f64::min);
    assert!((lowest - 9.8).abs() < 0.01, "{lowest}");
    // A layer that was not asked for such moves leaves them straight.
    let unasked = layer_of(&outline, &[], Needs::default());
    assert_eq!(unasked.route(from, to, Kind::Outside, None), vec![to]);
}

#[test]
fn a_layer_too_thin_to_have_an_inside_is_not_routed_on() {
    // A wall 0.4 mm thick: nothing is left of it behind its outer wall.
    let outline = [solid(rect(0.0, 0.0, 30.0, 0.4))];
    let layer = layer_of(&outline, &[], Needs::default());
    let (from, to) = (mm(1.0, 0.2), mm(29.0, 0.2));
    assert_eq!(layer.route(from, to, Kind::Part, None), vec![to]);
    // And one with nothing on it at all.
    let empty = layer_of(&[], &[], Needs::default());
    assert_eq!(empty.route(from, to, Kind::Part, None), vec![to]);
}
