use std::f64::consts::{PI, TAU};

use super::*;

/// A hole: the points at these angles and distances from a middle, in
/// mm, clockwise as a layer holds a hole.
fn hole_of(middle: (f64, f64), points: impl IntoIterator<Item = (f64, f64)>) -> Polygon {
    let mut points: Vec<Point2> = points
        .into_iter()
        .map(|(angle, reach)| {
            Point2::from_mm(
                middle.0 + reach * angle.cos(),
                middle.1 + reach * angle.sin(),
            )
        })
        .collect();
    points.reverse();
    Polygon { points }
}

/// A round hole of `sides` sides.
fn round(middle: (f64, f64), radius: f64, sides: usize) -> Polygon {
    hole_of(
        middle,
        (0..sides).map(|i| (TAU * i as f64 / sides as f64, radius)),
    )
}

/// A region 40 mm square about `middle` with these holes in it.
fn plate(middle: (f64, f64), holes: Vec<Polygon>) -> ExPolygon {
    let (x, y) = middle;
    ExPolygon {
        contour: Polygon {
            points: [(-20.0, -20.0), (20.0, -20.0), (20.0, 20.0), (-20.0, 20.0)]
                .map(|(dx, dy)| Point2::from_mm(x + dx, y + dy))
                .to_vec(),
        },
        holes,
    }
}

const MARGIN: Margin = Margin::Mm(0.01);

fn polyholes(twisted: bool) -> Polyholes {
    Polyholes {
        margin: MARGIN,
        twisted,
        max_edges: 50,
    }
}

/// The distance of a ring's corners from a middle, least and most, and
/// of the middles of its sides, in units.
fn reaches(ring: &Polygon, middle: Point2) -> ((f64, f64), (f64, f64)) {
    let span = |values: Vec<f64>| {
        (
            values.iter().copied().fold(f64::MAX, f64::min),
            values.iter().copied().fold(f64::MIN, f64::max),
        )
    };
    let points = &ring.points;
    let corners = points
        .iter()
        .map(|point| distance(*point, middle))
        .collect();
    let sides = (0..points.len())
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % points.len()]);
            (((a.x + b.x) as f64 / 2.0 - middle.x as f64).powi(2)
                + ((a.y + b.y) as f64 / 2.0 - middle.y as f64).powi(2))
            .sqrt()
        })
        .collect();
    (span(corners), span(sides))
}

#[test]
fn a_round_hole_is_told_by_its_points_and_its_sides() {
    let found = round_hole(&round((135.0, 136.0), 2.5, 64), MARGIN).unwrap();
    assert_eq!(found.centre, Point2::from_mm(135.0, 136.0));
    assert!((f64::from(found.radius) - 2_500_000.0).abs() < 2.0);
    assert_eq!(found.margin, 10_000);
    // A regular polygon of nine sides is as round as one of sixty-four.
    assert!(round_hole(&round((0.0, 0.0), 3.0, 9), MARGIN).is_some());
    // Eight points are too few, however regular: "more than 8".
    assert!(round_hole(&round((0.0, 0.0), 3.0, 8), MARGIN).is_none());
    assert!(round_hole(&round((0.0, 0.0), 3.0, 3), MARGIN).is_none());

    // An ellipse 0.2 mm longer than it is wide is not round.
    let ellipse = hole_of(
        (135.0, 136.0),
        (0..64).map(|i| {
            let angle = TAU * f64::from(i) / 64.0;
            (angle, 2.5 + 0.1 * (2.0 * angle).cos().abs())
        }),
    );
    assert!(round_hole(&ellipse, MARGIN).is_none());

    // Points all on one circle, but in four clusters: the corners are
    // at one distance and the middles of the long sides are not. "also
    // use center of lines to check it's not a rectangle".
    let clustered = hole_of(
        (0.0, 0.0),
        (0..4).flat_map(|corner| {
            (0..3).map(move |i| (PI / 2.0 * f64::from(corner) + 0.1 * f64::from(i), 3.0))
        }),
    );
    assert_eq!(clustered.points.len(), 12);
    assert!(round_hole(&clustered, MARGIN).is_none());
}

#[test]
fn the_margin_is_a_length_or_a_share_of_the_radius() {
    // A hole of 64 sides whose every other point is 0.03 mm further out.
    let wavy = |radius: f64| {
        hole_of(
            (10.0, 10.0),
            (0..64).map(move |i| {
                (
                    TAU * f64::from(i) / 64.0,
                    radius + if i % 2 == 0 { 0.0 } else { 0.03 },
                )
            }),
        )
    };
    // Its points stray by 0.03 mm, and every other one is a dent as
    // deep, so it is not convex either; a hole that is longer one way
    // than the other instead keeps to the outside of its neighbours.
    let bulging = |radius: f64, by: f64| {
        hole_of(
            (10.0, 10.0),
            (0..64).map(move |i| {
                let angle = TAU * f64::from(i) / 64.0;
                (angle, radius + by * (0.5 + 0.5 * (2.0 * angle).cos()))
            }),
        )
    };
    assert!(round_hole(&wavy(3.0), Margin::Mm(0.05)).is_none());
    // 0.03 mm of bulge: outside twice 0.01 mm, inside twice 0.02 mm.
    assert!(round_hole(&bulging(3.0, 0.03), Margin::Mm(0.01)).is_none());
    let found = round_hole(&bulging(3.0, 0.03), Margin::Mm(0.02)).unwrap();
    assert_eq!(found.margin, 20_000);
    // As a share of the mean radius: 1% of about 3 mm is about 0.03 mm.
    let found = round_hole(&bulging(3.0, 0.03), Margin::Percent(1.0)).unwrap();
    assert!((30_000..30_200).contains(&found.margin), "{}", found.margin);
    // The same bulge on a hole of a third the size is outside 1% of it.
    assert!(round_hole(&bulging(1.0, 0.03), Margin::Percent(1.0)).is_none());
    // No margin is less than the least distance told apart.
    let exact = round_hole(&round((0.0, 0.0), 3.0, 64), Margin::Mm(0.0)).unwrap();
    assert_eq!(exact.margin, 100);
    assert_eq!(
        round_hole(&round((0.0, 0.0), 3.0, 64), Margin::Mm(-1.0))
            .unwrap()
            .margin,
        100
    );
}

#[test]
fn a_dented_hole_is_left_alone_and_a_point_on_a_flat_is_no_dent() {
    // One point of 64 set in toward the middle of the hole. By 0.005 mm
    // it is still outside the line through its neighbours, and the hole
    // is convex. By 0.018 mm it is inside that line: a dent, though
    // every point is still within the margin of one distance. The
    // reference, whose test for a convex hole never fails, takes that
    // hole for round; here it is left alone.
    let with_point_at = |reach: f64| {
        hole_of(
            (50.0, 50.0),
            (0..64).map(move |i| (TAU * f64::from(i) / 64.0, if i == 10 { reach } else { 3.0 })),
        )
    };
    assert!(round_hole(&with_point_at(2.995), MARGIN).is_some());
    assert!(round_hole(&with_point_at(2.982), MARGIN).is_none());
    // It is the turn that fails it, not the distances: a point as far
    // out as that one is in passes.
    assert!(round_hole(&with_point_at(3.018), MARGIN).is_some());

    // A hole with a point in the middle of each flat, every other one a
    // unit to the wrong side, as rounding leaves them.
    let ring = round((50.0, 50.0), 6.0, 32);
    let mut points = Vec::new();
    for (index, &point) in ring.points.iter().enumerate() {
        let next = ring.points[(index + 1) % ring.points.len()];
        points.push(point);
        let (mx, my) = ((point.x + next.x) / 2, (point.y + next.y) / 2);
        // Toward the middle of the hole or away from it.
        let toward = if index % 2 == 0 { 1 } else { -1 };
        points.push(Point2 {
            x: mx + toward * (50_000_000 - mx).signum(),
            y: my + toward * (50_000_000 - my).signum(),
        });
    }
    let found = round_hole(&Polygon { points }, Margin::Mm(0.05));
    assert!(found.is_some());
}

#[test]
fn the_number_of_sides_comes_from_the_radius_and_the_nozzle() {
    let edges = |radius: f64, nozzle: f64, max: i32| {
        polyhole_edges(edge_geom::scaled(radius), scale(nozzle) as Coord, max)
    };
    // "n = max(round(2 * d), 3); // for 0.4mm nozzle"
    assert_eq!(edges(1.0, 0.4, 50), 4);
    assert_eq!(edges(1.5, 0.4, 50), 6);
    assert_eq!(edges(2.5, 0.4, 50), 10);
    assert_eq!(edges(4.0, 0.4, 50), 16);
    assert_eq!(edges(6.0, 0.4, 50), 24);
    // Never fewer than three.
    assert_eq!(edges(0.3, 0.4, 50), 3);
    // Fewer for a wider nozzle, more for a finer one.
    assert_eq!(edges(6.0, 0.6, 50), 16);
    assert_eq!(edges(6.0, 0.2, 50), 48);
    // And no more than the most allowed.
    assert_eq!(edges(20.0, 0.4, 50), 50);
    assert_eq!(edges(6.0, 0.4, 8), 8);
    assert_eq!(edges(6.0, 0.4, 3), 3);
}

#[test]
fn a_polyhole_has_its_flats_on_the_circle_and_is_clockwise() {
    let centre = Point2::from_mm(135.5, 128.0);
    for radius in [1.0, 1.5, 2.5, 4.0, 6.0] {
        let made = create_polyholes(centre, edge_geom::scaled(radius), 400_000, false, 50);
        assert_eq!(made.len(), 1);
        let ring = &made[0];
        assert_eq!(ring.points.len(), (4.0 * radius).round() as usize);
        assert!(!ring.is_ccw());
        let ((least, most), (least_side, most_side)) = reaches(ring, centre);
        // The middles of the sides are on the circle, to a few units
        // (the corners are worked out in `f32`).
        let units = radius * 1e6;
        assert!((least_side - units).abs() < 3.0 && (most_side - units).abs() < 3.0);
        // The corners are outside it, all by the same.
        let out = units / (PI / ring.points.len() as f64).cos();
        assert!((least - out).abs() < 3.0 && (most - out).abs() < 3.0);
    }
}

#[test]
fn a_twisted_polyhole_turns_two_fifths_of_a_side_a_layer() {
    let centre = Point2::from_mm(100.0, 100.0);
    let made = create_polyholes(centre, 6_000_000, 400_000, true, 50);
    assert_eq!(made.len(), 5);
    // Where the first corner laid out is (the last held, the ring
    // being turned over), in fifths of a side: 0, 2, 4, 1, 3.
    let side = TAU / 24.0;
    let turned: Vec<f64> = made
        .iter()
        .map(|ring| {
            let first = ring.points[ring.points.len() - 1];
            let angle = ((first.y - centre.y) as f64).atan2((first.x - centre.x) as f64);
            angle / side * 5.0
        })
        .collect();
    for (found, wanted) in turned.iter().zip([0.0, 2.0, 4.0, 1.0, 3.0]) {
        assert!((found - wanted).abs() < 1e-3, "{turned:?}");
    }
    for ring in &made {
        assert_eq!(ring.points.len(), 24);
        assert!(!ring.is_ccw());
    }
    // Not twisted: one polygon, where the first of the five is.
    let plain = create_polyholes(centre, 6_000_000, 400_000, false, 50);
    assert_eq!(plain, made[..1]);
}

/// Layers of one plate each, with these holes; converted in place.
fn converted(
    holes_by_layer: Vec<Vec<Polygon>>,
    of: &Polyholes,
    filament_of: impl Fn(usize) -> usize,
    nozzle: f64,
) -> (Vec<Vec<ExPolygon>>, Vec<Replaced>) {
    let mut layers: Vec<Vec<ExPolygon>> = holes_by_layer
        .into_iter()
        .map(|holes| vec![plate((100.0, 100.0), holes)])
        .collect();
    let mut shares: Vec<Vec<Share<'_>>> = layers
        .iter_mut()
        .enumerate()
        .map(|(index, regions)| {
            vec![Share {
                filament: filament_of(index),
                nozzle_diameter: nozzle,
                regions,
            }]
        })
        .collect();
    let replaced = hole_to_polyholes(&mut shares, of);
    drop(shares);
    (layers, replaced)
}

fn sides(layers: &[Vec<ExPolygon>]) -> Vec<Vec<usize>> {
    layers
        .iter()
        .map(|regions| {
            regions[0]
                .holes
                .iter()
                .map(|hole| hole.points.len())
                .collect()
        })
        .collect()
}

#[test]
fn a_hole_through_several_layers_is_converted_on_each() {
    let hole = || round((100.0, 100.0), 2.5, 64);
    let (layers, replaced) = converted(vec![vec![hole()]; 7], &polyholes(true), |_| 1, 0.4);
    assert_eq!(sides(&layers), vec![vec![10]; 7]);
    assert_eq!(replaced.len(), 7);
    assert_eq!(replaced[3].layer, 3);
    assert_eq!(replaced[3].from, hole());
    assert_eq!(replaced[3].to, layers[3][0].holes[0]);
    // Each layer has the polygon of its turn: the same again after five.
    let made = create_polyholes(Point2::from_mm(100.0, 100.0), 2_500_000, 400_000, true, 50);
    for (index, layer) in layers.iter().enumerate() {
        assert_eq!(layer[0].holes[0], made[index % 5], "layer {index}");
    }
    assert_ne!(layers[0][0].holes[0], layers[1][0].holes[0]);
    assert_eq!(layers[0][0].holes[0], layers[5][0].holes[0]);

    // Not twisted, every layer has the one polygon.
    let (flat, _) = converted(vec![vec![hole()]; 7], &polyholes(false), |_| 1, 0.4);
    assert!(flat.iter().all(|layer| layer[0].holes[0] == made[0]));

    // By the nozzle of the filament the hole is in.
    let (wide, _) = converted(vec![vec![hole()]; 3], &polyholes(true), |_| 1, 0.8);
    assert_eq!(sides(&wide), vec![vec![5]; 3]);
    // And no more sides than allowed.
    let capped = Polyholes {
        max_edges: 6,
        ..polyholes(true)
    };
    let (few, _) = converted(vec![vec![hole()]; 3], &capped, |_| 1, 0.4);
    assert_eq!(sides(&few), vec![vec![6]; 3]);
}

#[test]
fn a_hole_on_one_layer_only_is_converted_on_the_first_layer_alone() {
    let hole = |radius: f64| round((100.0, 100.0), radius, 64);
    // The first layer's hole is larger, as elephant foot compensation
    // leaves it, and is a run by itself; so is the second's, which is
    // left as drawn; the rest make a run of three.
    let (layers, replaced) = converted(
        vec![
            vec![hole(2.65)],
            vec![hole(2.58)],
            vec![hole(2.5)],
            vec![hole(2.5)],
            vec![hole(2.5)],
        ],
        &polyholes(true),
        |_| 1,
        0.4,
    );
    assert_eq!(
        sides(&layers),
        vec![vec![11], vec![64], vec![10], vec![10], vec![10]]
    );
    assert_eq!(replaced.len(), 4);
    // Each run's polygon is drawn round its own first hole.
    let ((_, _), (flat, _)) = reaches(&layers[0][0].holes[0], Point2::from_mm(100.0, 100.0));
    assert!((flat - 2_650_000.0).abs() < 3.0);

    // A hole that moves by more than the margin from layer to layer is
    // no run at all, and nothing above the first layer is converted.
    let leaning: Vec<Vec<Polygon>> = (0..4)
        .map(|index| vec![round((100.0 + 0.02 * f64::from(index), 100.0), 2.5, 64)])
        .collect();
    let (layers, _) = converted(leaning, &polyholes(true), |_| 1, 0.4);
    assert_eq!(sides(&layers), vec![vec![10], vec![64], vec![64], vec![64]]);

    // One that keeps within the margin of where it started is one run,
    // drawn where it started.
    let wobbling: Vec<Vec<Polygon>> = (0..4)
        .map(|index| {
            vec![round(
                (100.0 + 0.004 * f64::from(index % 2), 100.0),
                2.5,
                64,
            )]
        })
        .collect();
    let (layers, _) = converted(wobbling, &polyholes(false), |_| 1, 0.4);
    assert_eq!(sides(&layers), vec![vec![10]; 4]);
    assert!(
        layers
            .iter()
            .all(|layer| layer[0].holes[0] == layers[0][0].holes[0])
    );
}

#[test]
fn runs_are_kept_apart_by_layer_filament_and_place() {
    let hole = || round((100.0, 100.0), 2.5, 64);
    let other = || round((110.0, 100.0), 4.0, 64);
    // A square hole between two stretches of a round one: two runs.
    // The reference would convert only the first, its runs being kept
    // by middle and radius; here both are.
    let square = || round((100.0, 100.0), 2.5, 4);
    let (layers, _) = converted(
        vec![
            vec![hole(), other()],
            vec![hole(), other()],
            vec![square(), other()],
            vec![hole(), other()],
            vec![hole(), other()],
        ],
        &polyholes(true),
        |_| 1,
        0.4,
    );
    assert_eq!(
        sides(&layers),
        vec![
            vec![10, 16],
            vec![10, 16],
            vec![4, 16],
            vec![10, 16],
            vec![10, 16]
        ]
    );
    // A hole that is in another filament on the next layer is not the
    // same hole: neither layer above the first has a run of two.
    let (layers, _) = converted(
        vec![vec![hole()], vec![hole()], vec![hole()]],
        &polyholes(true),
        |index| index % 2 + 1,
        0.4,
    );
    assert_eq!(sides(&layers), vec![vec![10], vec![64], vec![64]]);
}
