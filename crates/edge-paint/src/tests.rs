use edge_geom::{Point2, Polygon, scaled};

use super::*;

/// A block's twelve triangles, each side painted as `paint` says of it:
/// 0 and 1 are the sides at the least and greatest x, 2 and 3 those at
/// the least and greatest y, 4 the bottom and 5 the top.
fn block(low: [f32; 3], high: [f32; 3], paint: impl Fn(usize) -> u8) -> Vec<Facet> {
    let [x0, y0, z0] = low;
    let [x1, y1, z1] = high;
    let sides: [[[f32; 3]; 4]; 6] = [
        [[x0, y0, z0], [x0, y0, z1], [x0, y1, z1], [x0, y1, z0]],
        [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
        [[x0, y0, z0], [x1, y0, z0], [x1, y0, z1], [x0, y0, z1]],
        [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
        [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
        [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
    ];
    sides
        .iter()
        .enumerate()
        .flat_map(|(side, [a, b, c, d])| {
            [[*a, *b, *c], [*a, *c, *d]].map(|corners| Facet {
                corners,
                filament: paint(side),
            })
        })
        .collect()
}

fn square(low: f64, high: f64) -> Vec<ExPolygon> {
    vec![ExPolygon {
        contour: Polygon {
            points: [(low, low), (high, low), (high, high), (low, high)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        },
        holes: Vec::new(),
    }]
}

fn settings(top_layers: usize, bottom_layers: usize) -> PaintSettings {
    PaintSettings {
        top_layers,
        bottom_layers,
        wall_width: scaled(0.5),
        wall_spacing: scaled(0.5),
        small: scaled(0.1),
    }
}

/// Ten layers of a 10 mm block, 0.2 mm each, cut through their middles.
fn layers() -> (Vec<Vec<ExPolygon>>, Vec<f64>) {
    (
        vec![square(0.0, 10.0); 10],
        (0..10).map(|layer| 0.1 + 0.2 * f64::from(layer)).collect(),
    )
}

/// Each layer's painted area by filament, in mm².
fn areas(painted: &[Vec<(u8, Vec<ExPolygon>)>]) -> Vec<Vec<(u8, f64)>> {
    painted
        .iter()
        .map(|layer| {
            layer
                .iter()
                .map(|(filament, regions)| {
                    let area: f64 = regions.iter().map(ExPolygon::area_mm2).sum();
                    (*filament, (area * 10.0).round() / 10.0)
                })
                .collect()
        })
        .collect()
}

#[test]
fn a_painted_side_takes_what_lies_nearest_it() {
    let (outlines, planes) = layers();
    let outlines: Vec<&[ExPolygon]> = outlines.iter().map(Vec::as_slice).collect();
    // The side at the greatest x for filament 2, the one at the least y
    // for filament 5, the rest bare. No shells, so only the sides count.
    let facets = block([0.0; 3], [10.0, 10.0, 2.0], |side| match side {
        1 => 2,
        2 => 5,
        _ => 0,
    });
    let painted = segment(&outlines, &planes, &facets, &settings(0, 0)).unwrap();
    // Each side's share is the triangle from it to the middle: a quarter.
    for layer in areas(&painted) {
        assert_eq!(layer, [(2, 25.0), (5, 25.0)]);
    }
    // The filament-2 share lies against its own side.
    let (_, share) = &painted[4][0];
    assert!(
        share
            .iter()
            .flat_map(|region| &region.contour.points)
            .all(|point| point.x >= scaled(4.99))
    );

    // Nothing painted, nothing moved.
    let bare = block([0.0; 3], [10.0, 10.0, 2.0], |_| 0);
    let painted = segment(&outlines, &planes, &bare, &settings(3, 3)).unwrap();
    assert!(painted.iter().all(Vec::is_empty));
}

#[test]
fn one_colour_all_round_takes_the_whole_layer() {
    let (outlines, planes) = layers();
    let outlines: Vec<&[ExPolygon]> = outlines.iter().map(Vec::as_slice).collect();
    let facets = block(
        [0.0; 3],
        [10.0, 10.0, 2.0],
        |side| if side < 4 { 3 } else { 0 },
    );
    let painted = segment(&outlines, &planes, &facets, &settings(0, 0)).unwrap();
    for layer in areas(&painted) {
        assert_eq!(layer, [(3, 100.0)]);
    }
}

#[test]
fn a_painted_top_goes_down_through_the_shell_and_no_further() {
    let (outlines, planes) = layers();
    let outlines: Vec<&[ExPolygon]> = outlines.iter().map(Vec::as_slice).collect();
    let facets = block(
        [0.0; 3],
        [10.0, 10.0, 2.0],
        |side| if side == 5 { 4 } else { 0 },
    );
    let painted = areas(&segment(&outlines, &planes, &facets, &settings(3, 0)).unwrap());
    // The top layer whole; the two behind it in by a wall and a space,
    // which is as far in as an open face is ever drawn; nothing below.
    assert_eq!(painted[9], [(4, 100.0)]);
    assert_eq!(painted[8], [(4, 64.0)]);
    assert_eq!(painted[7], [(4, 64.0)]);
    assert!(painted[..7].iter().all(Vec::is_empty));

    // A bottom goes up the same way, and takes the first layer.
    let facets = block(
        [0.0; 3],
        [10.0, 10.0, 2.0],
        |side| if side == 4 { 6 } else { 0 },
    );
    let painted = areas(&segment(&outlines, &planes, &facets, &settings(0, 2)).unwrap());
    assert_eq!(painted[0], [(6, 100.0)]);
    assert_eq!(painted[1], [(6, 64.0)]);
    assert!(painted[2..].iter().all(Vec::is_empty));
}

#[test]
fn a_face_wins_over_the_sides_on_the_layers_it_reaches() {
    let (outlines, planes) = layers();
    let outlines: Vec<&[ExPolygon]> = outlines.iter().map(Vec::as_slice).collect();
    // Sides for filament 1, top for filament 2, bottom left bare.
    let facets = block([0.0; 3], [10.0, 10.0, 2.0], |side| match side {
        5 => 2,
        4 => 0,
        _ => 1,
    });
    let painted = areas(&segment(&outlines, &planes, &facets, &settings(2, 2)).unwrap());
    // On top the face has it all; a layer down it has the middle and
    // the sides the rim.
    assert_eq!(painted[9], [(2, 100.0)]);
    assert_eq!(painted[8], [(1, 36.0), (2, 64.0)]);
    // The bare bottom keeps the part's own filament where it reaches:
    // all of the first layer, the middle of the second.
    assert!(painted[0].is_empty());
    assert_eq!(painted[1], [(1, 36.0)]);
    // Between the two the sides have everything.
    assert_eq!(painted[4], [(1, 100.0)]);
}
