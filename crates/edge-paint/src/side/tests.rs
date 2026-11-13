use edge_geom::{ExPolygon, Point2, Polygon};

use super::*;
use crate::{Facet, PaintSettings, segment};

/// The two triangles of a quad given counter-clockwise from outside.
fn quad(corners: [[f32; 3]; 4], filament: u8) -> [Facet; 2] {
    let [a, b, c, d] = corners;
    [
        Facet {
            corners: [a, b, c],
            filament,
        },
        Facet {
            corners: [a, c, d],
            filament,
        },
    ]
}

/// The sides of a 10 mm square prism 2 mm tall, each side cut into
/// upright strips: `strips` gives, for each side (least y, greatest x,
/// greatest y, least x, going round), where along it the strips start,
/// from 0 to 10, and their colours.
fn prism(strips: [&[(f32, u8)]; 4]) -> Vec<Facet> {
    let corner = |side: usize, at: f32| -> [f32; 2] {
        match side {
            0 => [at, 0.0],
            1 => [10.0, at],
            2 => [10.0 - at, 10.0],
            _ => [0.0, 10.0 - at],
        }
    };
    let mut facets = Vec::new();
    for (side, runs) in strips.iter().enumerate() {
        for (at, &(from, colour)) in runs.iter().enumerate() {
            let to = runs.get(at + 1).map_or(10.0, |next| next.0);
            let ([x0, y0], [x1, y1]) = (corner(side, from), corner(side, to));
            facets.extend(quad(
                [[x0, y0, 0.0], [x1, y1, 0.0], [x1, y1, 2.0], [x0, y0, 2.0]],
                colour,
            ));
        }
    }
    facets
}

fn ring(points: &[(f64, f64)]) -> Polygon {
    Polygon {
        points: points.iter().map(|&(x, y)| Point2::from_mm(x, y)).collect(),
    }
}

fn square() -> Vec<ExPolygon> {
    vec![ExPolygon {
        contour: ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
        holes: Vec::new(),
    }]
}

/// The shares of one layer at 1 mm up, sides only, in mm² by colour.
fn shares(outline: &[ExPolygon], facets: &[Facet]) -> Vec<(u8, f64)> {
    let settings = PaintSettings {
        top_layers: 0,
        bottom_layers: 0,
        wall_width: edge_geom::scaled(0.45),
        wall_spacing: edge_geom::scaled(0.4),
        small: edge_geom::scaled(0.1),
        run_scale: 1.0,
    };
    let painted = segment(&[outline], &[1.0], facets, &settings).unwrap();
    painted[0]
        .iter()
        .map(|(colour, regions)| {
            let area: f64 = regions.iter().map(ExPolygon::area_mm2).sum();
            (*colour, (area * 100.0).round() / 100.0)
        })
        .collect()
}

#[test]
fn two_colours_split_down_the_middle_take_a_half_each() {
    // The left half of the prism in 1, the right in 2: the front and
    // back are split at x = 5.
    let facets = prism([
        &[(0.0, 1), (5.0, 2)],
        &[(0.0, 2)],
        &[(0.0, 2), (5.0, 1)],
        &[(0.0, 1)],
    ]);
    assert_eq!(shares(&square(), &facets), [(1, 50.0), (2, 50.0)]);
}

#[test]
fn three_colours_meet_in_the_middle() {
    // The front in 1, the right in 2, the back and left in 3: each side
    // takes the triangle from it to the middle.
    let facets = prism([&[(0.0, 1)], &[(0.0, 2)], &[(0.0, 3)], &[(0.0, 3)]]);
    assert_eq!(
        shares(&square(), &facets),
        [(1, 25.0), (2, 25.0), (3, 50.0)]
    );
}

#[test]
fn a_stripe_narrower_than_a_wall_keeps_its_strip_unless_it_is_a_scrap() {
    // 0.3 mm of colour 2 in the middle of the front: more than the
    // 0.2 mm a run of colour has to have, so it takes what lies nearest
    // it, a strip 0.3 mm wide that runs in until the sides' triangles
    // close it off.
    let facets = prism([
        &[(0.0, 0), (4.85, 2), (5.15, 0)],
        &[(0.0, 0)],
        &[(0.0, 0)],
        &[(0.0, 0)],
    ]);
    let got = shares(&square(), &facets);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, 2);
    assert!((got[0].1 - 0.3 * 4.85).abs() < 0.05, "{got:?}");
    // 0.15 mm goes to the unpainted run about it.
    let facets = prism([
        &[(0.0, 0), (4.925, 2), (5.075, 0)],
        &[(0.0, 0)],
        &[(0.0, 0)],
        &[(0.0, 0)],
    ]);
    assert!(shares(&square(), &facets).is_empty());
    // And 0.4 mm of nothing between two runs of 2 is closed over.
    let facets = prism([
        &[(0.0, 2), (4.8, 0), (5.2, 2)],
        &[(0.0, 0)],
        &[(0.0, 0)],
        &[(0.0, 0)],
    ]);
    assert_eq!(shares(&square(), &facets), [(2, 25.0)]);
}

#[test]
fn a_hole_in_one_colour_and_an_outside_in_another_share_the_ring() {
    // A 10 mm square with a 4 mm hole in its middle, the outside in 1
    // and the hole's sides in 2: each takes what is nearer it. Neither
    // ring changes colour, so each puts in one border of its own.
    let outline = vec![ExPolygon {
        contour: ring(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]),
        holes: vec![ring(&[(3.0, 3.0), (3.0, 7.0), (7.0, 7.0), (7.0, 3.0)])],
    }];
    let mut facets = prism([&[(0.0, 1)], &[(0.0, 1)], &[(0.0, 1)], &[(0.0, 1)]]);
    for [(x0, y0), (x1, y1)] in [
        [(3.0, 3.0), (3.0, 7.0)],
        [(3.0, 7.0), (7.0, 7.0)],
        [(7.0, 7.0), (7.0, 3.0)],
        [(7.0, 3.0), (3.0, 3.0)],
    ] {
        facets.extend(quad(
            [[x0, y0, 0.0], [x1, y1, 0.0], [x1, y1, 2.0], [x0, y0, 2.0]],
            2,
        ));
    }
    let got = shares(&outline, &facets);
    // Halfway between the two rings, 1.5 mm from each, along the
    // hole's sides: 24 mm². Beyond its corners the points nearer a
    // corner of the hole than the outside, under a parabola from 1.5 mm
    // out along each side to 1.24 mm out along the diagonal: 1.97 mm²
    // a corner. The border is drawn as the diagram's edges between
    // their vertices, straight, as the reference draws it, which cuts
    // 0.053 mm² off under each of the eight arcs.
    let one = got.iter().find(|(c, _)| *c == 1).unwrap().1;
    let two = got.iter().find(|(c, _)| *c == 2).unwrap().1;
    assert!((one + two - 84.0).abs() < 0.01, "{got:?}");
    assert!(
        (two - (24.0 + 4.0 * 1.9706 - 8.0 * 0.0533)).abs() < 0.02,
        "{got:?}"
    );
}

#[test]
fn only_triangles_across_the_plane_are_cut_and_flat_ones_never() {
    // A painted top is flat and colours no layer's outline; a side
    // triangle lying with an edge in the plane is cut along that edge.
    let top = quad(
        [
            [0.0, 0.0, 2.0],
            [10.0, 0.0, 2.0],
            [10.0, 10.0, 2.0],
            [0.0, 10.0, 2.0],
        ],
        2,
    );
    assert!(cuts(&top, &[1.0, 2.0]).iter().all(Vec::is_empty));
    let side = [Facet {
        corners: [[0.0, 0.0, 1.0], [10.0, 0.0, 1.0], [10.0, 0.0, 2.0]],
        filament: 3,
    }];
    let found = cuts(&side, &[1.0]);
    assert_eq!(found[0].len(), 1);
    let line = found[0][0].line;
    let (mut xs, ys) = ([line.a.x, line.b.x], [line.a.y, line.b.y]);
    xs.sort_unstable();
    assert_eq!((xs, ys), ([0, 10_000_000], [0, 0]));
}

#[test]
fn a_layer_painted_all_round_in_one_colour_is_that_colours_whole() {
    let facets = prism([&[(0.0, 4)], &[(0.0, 4)], &[(0.0, 4)], &[(0.0, 4)]]);
    assert_eq!(shares(&square(), &facets), [(4, 100.0)]);
}
