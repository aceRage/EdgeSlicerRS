use std::f64::consts::TAU;

use super::*;
use crate::rings::distance;

fn ring(middle: (f64, f64), sides: usize, reach: impl Fn(f64) -> f64) -> Polygon {
    Polygon {
        points: (0..sides)
            .map(|i| {
                let angle = TAU * i as f64 / sides as f64;
                Point2::from_mm(
                    middle.0 + reach(angle) * angle.cos(),
                    middle.1 + reach(angle) * angle.sin(),
                )
            })
            .collect(),
    }
}

fn hole(middle: (f64, f64), sides: usize, radius: f64) -> Polygon {
    let mut hole = ring(middle, sides, |_| radius);
    hole.points.reverse();
    hole
}

/// The furthest a ring's points are from a middle, in mm.
fn radius(ring: &Polygon, middle: (f64, f64)) -> f64 {
    let middle = Point2::from_mm(middle.0, middle.1);
    ring.points
        .iter()
        .map(|point| distance(*point, middle))
        .fold(0.0, f64::max)
        / 1e6
}

#[test]
fn a_circle_is_eight_points_or_more_at_one_distance() {
    let found = approx_circle(&ring((135.0, 136.0), 64, |_| 5.0).points).unwrap();
    assert_eq!(found.centre, Point2::from_mm(135.0, 136.0));
    assert!((found.diameter - 10_000_000.0).abs() < 2.0);
    // Either way round.
    assert!(approx_circle(&hole((0.0, 0.0), 64, 5.0).points).is_some());
    // Eight sides will do, seven will not.
    assert!(approx_circle(&ring((0.0, 0.0), 8, |_| 3.0).points).is_some());
    assert!(approx_circle(&ring((0.0, 0.0), 7, |_| 3.0).points).is_none());
    // Too much spread: an ellipse 0.2 mm longer than wide.
    let ellipse = ring((0.0, 0.0), 64, |angle| 2.5 + 0.1 * (2.0 * angle).cos());
    assert!(approx_circle(&ellipse.points).is_none());
    // A little spread is let through: 0.02 mm of it.
    let nearly = ring((0.0, 0.0), 64, |angle| 2.5 + 0.01 * (2.0 * angle).cos());
    assert!(approx_circle(&nearly.points).is_some());
    // One point far out among many is little spread, but is more than
    // half a millimetre from the rest.
    let spiked = ring(
        (0.0, 0.0),
        400,
        |angle| if angle == 0.0 { 5.6 } else { 5.0 },
    );
    assert!(approx_circle(&spiked.points).is_none());
    // Points along the flats of an octagon are too spread to be one.
    let octagon = ring((0.0, 0.0), 8, |_| 3.0);
    let mut resampled = Vec::new();
    for (index, &from) in octagon.points.iter().enumerate() {
        let to = octagon.points[(index + 1) % 8];
        for step in 0..4 {
            resampled.push(Point2 {
                x: from.x + (to.x - from.x) * step / 4,
                y: from.y + (to.y - from.y) * step / 4,
            });
        }
    }
    assert!(approx_circle(&resampled).is_none());
}

#[test]
fn the_amount_is_a_line_in_speed_and_diameter_held_between_limits() {
    let of = CircleCompensation::default();
    let hole =
        |diameter: f64| compensation(of.hole_coefs, of.hole_limits, of.speed, diameter * 1e6) / 1e6;
    let contour = |diameter: f64| {
        compensation(
            of.contour_coefs,
            of.contour_limits,
            of.speed,
            diameter * 1e6,
        ) / 1e6
    };
    // Holes: 0.28 - 0.025 d, between 0.08 and 0.25.
    assert!((hole(2.0) - 0.23).abs() < 1e-9);
    assert!((hole(5.0) - 0.155).abs() < 1e-9);
    assert!((hole(0.5) - 0.25).abs() < 1e-9);
    assert!((hole(12.0) - 0.08).abs() < 1e-9);
    // Outlines: 0.025 d - 0.11, between -0.04 and 0.05.
    assert!((contour(5.0) - 0.015).abs() < 1e-9);
    assert!((contour(2.0) + 0.04).abs() < 1e-9);
    assert!((contour(10.0) - 0.05).abs() < 1e-9);
    // The first coefficient is per mm/s of the compensation speed.
    let with_speed = compensation([0.0005, 0.0, 0.0], (-1.0, 1.0), 200.0, 5e6) / 1e6;
    assert!((with_speed - 0.1).abs() < 1e-9);
    // Limits the wrong way round: the upper one is both.
    let crossed = compensation([0.0, 0.0, 0.3], (0.5, 0.1), 200.0, 5e6) / 1e6;
    assert!((crossed - 0.1).abs() < 1e-9);
}

/// A 40 mm plate about (100, 100) with a 5 mm hole and a 12 mm hole,
/// and beside it a round boss 10 mm across.
fn regions() -> Vec<ExPolygon> {
    vec![
        ExPolygon {
            contour: Polygon {
                points: [(80.0, 80.0), (120.0, 80.0), (120.0, 120.0), (80.0, 120.0)]
                    .map(|(x, y)| Point2::from_mm(x, y))
                    .to_vec(),
            },
            holes: vec![hole((90.0, 100.0), 64, 2.5), hole((108.0, 100.0), 64, 6.0)],
        },
        ExPolygon {
            contour: ring((140.0, 100.0), 64, |_| 5.0),
            holes: vec![hole((140.0, 100.0), 64, 1.0)],
        },
    ]
}

#[test]
fn round_holes_are_opened_and_round_outlines_moved() {
    let of = CircleCompensation::default();
    let mut made = regions();
    let (marks, replaced) = auto_circle_compensation(7, &mut made, &of, 0.0).unwrap();
    // Corners move out by the amount over the cosine of half a turn;
    // on 64 sides that is 0.12% more.
    let near = |found: f64, wanted: f64| (found - wanted).abs() < 0.0005;
    // 5 mm hole: 0.155 larger in radius. 12 mm hole: 0.08. 2 mm: 0.23.
    assert!(near(radius(&made[0].holes[0], (90.0, 100.0)), 2.655));
    assert!(near(radius(&made[0].holes[1], (108.0, 100.0)), 6.08));
    assert!(near(radius(&made[1].holes[0], (140.0, 100.0)), 1.23));
    // 10 mm boss: 0.025 * 10 - 0.11 = 0.14, held to 0.05.
    assert!(near(radius(&made[1].contour, (140.0, 100.0)), 5.05));
    // The square is no circle, and is as it was.
    assert_eq!(made[0].contour, regions()[0].contour);
    // Holes stay clockwise and outlines counter-clockwise.
    assert!(made.iter().all(|region| region.contour.is_ccw()));
    assert!(
        made.iter()
            .flat_map(|region| &region.holes)
            .all(|hole| !hole.is_ccw())
    );
    // Everything resized is under 50 mm across, and marked.
    assert_eq!(
        marks,
        vec![
            Marked {
                contour: false,
                holes: vec![0, 1]
            },
            Marked {
                contour: true,
                holes: vec![0]
            }
        ]
    );
    // What was put in place of what, with the layer it was on.
    assert_eq!(replaced.len(), 4);
    assert!(replaced.iter().all(|change| change.layer == 7));
    assert_eq!(replaced[0].from, regions()[0].holes[0]);
    assert_eq!(replaced[0].to, made[0].holes[0]);
    assert_eq!(replaced[2].from, regions()[1].contour);
}

#[test]
fn the_manual_offset_takes_half_of_itself_off_either_side() {
    let of = CircleCompensation::default();
    let near = |found: f64, wanted: f64| (found - wanted).abs() < 0.0005;
    // Positive: holes larger, outlines smaller, by half each.
    let mut looser = regions();
    auto_circle_compensation(0, &mut looser, &of, 0.1).unwrap();
    assert!(near(radius(&looser[0].holes[0], (90.0, 100.0)), 2.705));
    assert!(near(radius(&looser[1].contour, (140.0, 100.0)), 5.0));
    // Negative: the other way, and past nothing: a hole made smaller.
    let mut tighter = regions();
    auto_circle_compensation(0, &mut tighter, &of, -0.4).unwrap();
    assert!(near(radius(&tighter[0].holes[0], (90.0, 100.0)), 2.455));
    assert!(near(radius(&tighter[1].contour, (140.0, 100.0)), 5.25));
}

#[test]
fn a_circle_at_the_diameter_limit_is_resized_and_not_marked() {
    let of = CircleCompensation {
        diameter_limit: 9.9,
        ..CircleCompensation::default()
    };
    let mut made = regions();
    let (marks, replaced) = auto_circle_compensation(0, &mut made, &of, 0.0).unwrap();
    // The 12 mm hole and the 10 mm boss are resized all the same.
    assert_eq!(replaced.len(), 4);
    assert!((radius(&made[0].holes[1], (108.0, 100.0)) - 6.08).abs() < 0.0005);
    assert_eq!(marks[0].holes, vec![0]);
    // 10 mm is not under 9.9 mm.
    assert!(!marks[1].contour);
    assert_eq!(marks[1].holes, vec![0]);
}

#[test]
fn the_filaments_own_figures_are_followed() {
    // Bambu PLA Basic on an H2S, as the reference ships it.
    let of = CircleCompensation {
        speed: 200.0,
        contour_coefs: [0.0, 0.0025, 0.014],
        hole_coefs: [0.0, -0.0028, 0.12],
        contour_limits: (0.014, 0.076),
        hole_limits: (0.05, 0.12),
        diameter_limit: 50.0,
    };
    let mut made = regions();
    auto_circle_compensation(0, &mut made, &of, 0.0).unwrap();
    let near = |found: f64, wanted: f64| (found - wanted).abs() < 0.0005;
    // 0.12 - 0.0028 * 5 = 0.106; 0.014 + 0.0025 * 10 = 0.039.
    assert!(near(radius(&made[0].holes[0], (90.0, 100.0)), 2.606));
    assert!(near(radius(&made[1].contour, (140.0, 100.0)), 5.039));
}
