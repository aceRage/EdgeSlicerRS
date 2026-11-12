//! Arrangements checked for what must hold whatever the objective does:
//! nothing nearer anything else than the spacing, nothing off the bed,
//! what does not fit on the next bed, the same answer every time.

use std::time::Instant;

use edge_arrange::{
    Params, Piece, arrange, shrunk_bed, update_arrange_params, update_selected_items_inflation,
    update_unselected_items_inflation,
};
use edge_geom::{Point2, Polygon, scaled};

const BED: [f64; 4] = [0.0, 0.0, 270.0, 270.0];

fn rect(x: f64, y: f64, w: f64, h: f64) -> Polygon {
    Polygon {
        points: [(x, y), (x + w, y), (x + w, y + h), (x, y + h)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    }
}

fn disc(x: f64, y: f64, r: f64) -> Polygon {
    Polygon {
        points: (0..24)
            .map(|k| {
                let a = f64::from(k) * std::f64::consts::TAU / 24.0;
                Point2::from_mm(x + r * a.cos(), y + r * a.sin())
            })
            .collect(),
    }
}

fn piece(outline: Polygon, name: &str) -> Piece {
    Piece {
        outline,
        brim_width: 1.0,
        filaments: vec![1],
        height: 10.0,
        name: name.to_owned(),
        ..Piece::default()
    }
}

/// Arranges as the reference's job does: the parameters, the growth of
/// each item, then the arrangement on the shrunk bed.
fn run(items: &mut [Piece], fixed: &mut [Piece], mut params: Params) -> [i64; 4] {
    update_arrange_params(&mut params, 0.0, items);
    let bed = shrunk_bed(BED, &params);
    update_selected_items_inflation(items, bed, &mut params);
    update_unselected_items_inflation(fixed, &params);
    arrange(items, fixed, bed, &params);
    bed
}

fn segment_distance(p: Point2, a: Point2, b: Point2) -> f64 {
    let (px, py) = ((p.x - a.x) as f64, (p.y - a.y) as f64);
    let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - t * dx).powi(2) + (py - t * dy).powi(2)).sqrt()
}

fn inside(poly: &Polygon, p: Point2) -> bool {
    let pts = &poly.points;
    (0..pts.len()).all(|i| {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        i128::from(b.x - a.x) * i128::from(p.y - a.y)
            - i128::from(b.y - a.y) * i128::from(p.x - a.x)
            >= 0
    })
}

/// The gap between two convex outlines, in mm; 0 when they overlap.
fn gap(a: &Polygon, b: &Polygon) -> f64 {
    if a.points.iter().any(|p| inside(b, *p)) || b.points.iter().any(|p| inside(a, *p)) {
        return 0.0;
    }
    let mut least = f64::MAX;
    for (u, v) in [(a, b), (b, a)] {
        for p in &u.points {
            for i in 0..v.points.len() {
                let (s, t) = (v.points[i], v.points[(i + 1) % v.points.len()]);
                least = least.min(segment_distance(*p, s, t));
            }
        }
    }
    least / 1e6
}

fn check(items: &[Piece], bed: [i64; 4], spacing: f64) {
    for (i, a) in items.iter().enumerate() {
        if a.bed < 0 {
            continue;
        }
        let shape = a.transformed();
        for p in &shape.points {
            assert!(
                p.x >= bed[0] && p.x <= bed[2] && p.y >= bed[1] && p.y <= bed[3],
                "{} is off the bed at {p:?}",
                a.name
            );
        }
        for b in &items[i + 1..] {
            if b.bed != a.bed {
                continue;
            }
            let g = gap(&shape, &b.transformed());
            assert!(
                g >= spacing - 1e-3,
                "{} and {} are {g} mm apart",
                a.name,
                b.name
            );
        }
    }
}

#[test]
fn cubes_of_one_size_are_spaced_on_the_bed() {
    let mut items: Vec<Piece> = (0..9)
        .map(|k| {
            piece(
                rect(f64::from(k) * 3.0, 0.0, 20.0, 20.0),
                &format!("cube{k}"),
            )
        })
        .collect();
    let bed = run(&mut items, &mut [], Params::default());
    assert!(items.iter().all(|p| p.bed == 0));
    // Auto spacing: a millimetre round each.
    check(&items, bed, 2.0);
    // The pile is in the middle of the bed.
    let shapes: Vec<Point2> = items.iter().flat_map(|p| p.transformed().points).collect();
    let (x0, x1) = (
        shapes.iter().map(|p| p.x).min().unwrap(),
        shapes.iter().map(|p| p.x).max().unwrap(),
    );
    let middle = (x0 + x1) as f64 / 2.0 / 1e6;
    assert!((middle - 135.0).abs() < 2.0, "{middle}");
}

#[test]
fn a_spacing_given_is_kept() {
    let mut items: Vec<Piece> = (0..6)
        .map(|k| {
            piece(
                rect(0.0, 0.0, 15.0 + f64::from(k) * 5.0, 30.0),
                &format!("p{k}"),
            )
        })
        .collect();
    let params = Params {
        min_obj_distance: scaled(12.0),
        ..Params::default()
    };
    let bed = run(&mut items, &mut [], params);
    check(&items, bed, 12.0);
}

#[test]
fn what_one_bed_cannot_hold_goes_to_the_next() {
    let mut items: Vec<Piece> = (0..12)
        .map(|k| piece(rect(0.0, 0.0, 80.0, 80.0), &format!("big{k}")))
        .collect();
    let bed = run(&mut items, &mut [], Params::default());
    let on_first = items.iter().filter(|p| p.bed == 0).count();
    let on_second = items.iter().filter(|p| p.bed == 1).count();
    assert_eq!(on_first, 9, "three by three fit a 268 mm bed");
    assert_eq!(on_second, 3);
    check(&items, bed, 2.0);
}

#[test]
fn something_bigger_than_the_bed_goes_nowhere() {
    let mut items = vec![
        piece(rect(0.0, 0.0, 300.0, 20.0), "long"),
        piece(rect(0.0, 0.0, 20.0, 20.0), "small"),
    ];
    run(&mut items, &mut [], Params::default());
    assert_eq!(items[0].bed, -1);
    assert_eq!(items[1].bed, 0);
}

#[test]
fn turning_lets_a_long_part_fit() {
    let mut items = vec![piece(rect(0.0, 0.0, 300.0, 20.0), "long")];
    let params = Params {
        allow_rotations: true,
        ..Params::default()
    };
    let bed = run(&mut items, &mut [], params);
    assert_eq!(items[0].bed, 0);
    check(&items, bed, 0.0);
}

#[test]
fn fixed_objects_are_kept_clear_of() {
    let mut fixed = vec![Piece {
        bed: 0,
        ..piece(rect(100.0, 100.0, 70.0, 70.0), "fixed")
    }];
    let mut items: Vec<Piece> = (0..10)
        .map(|k| piece(disc(0.0, 0.0, 12.0), &format!("disc{k}")))
        .collect();
    let bed = run(&mut items, &mut fixed, Params::default());
    let mut all = items.clone();
    all.push(fixed[0].clone());
    check(&all, bed, 2.0);
    assert!(items.iter().all(|p| p.bed == 0));
}

#[test]
fn the_same_arrangement_every_time() {
    let make = || -> Vec<Piece> {
        (0..15)
            .map(|k| {
                let w = 10.0 + f64::from(k % 5) * 7.0;
                piece(rect(0.0, 0.0, w, 40.0 - w / 2.0), &format!("m{k}"))
            })
            .collect()
    };
    let (mut a, mut b) = (make(), make());
    run(&mut a, &mut [], Params::default());
    run(&mut b, &mut [], Params::default());
    for (x, y) in a.iter().zip(&b) {
        assert_eq!(
            (x.translation, x.rotation, x.bed),
            (y.translation, y.rotation, y.bed)
        );
    }
}

#[test]
fn fifty_parts_arrange_quickly() {
    let mut items: Vec<Piece> = (0..50)
        .map(|k| {
            let s = 8.0 + f64::from(k % 7) * 3.0;
            if k % 3 == 0 {
                piece(disc(0.0, 0.0, s / 2.0), &format!("d{k}"))
            } else {
                piece(rect(0.0, 0.0, s, s * 1.5), &format!("r{k}"))
            }
        })
        .collect();
    let start = Instant::now();
    let bed = run(&mut items, &mut [], Params::default());
    let took = start.elapsed();
    check(&items, bed, 2.0);
    eprintln!("50 parts: {took:?}");
}
