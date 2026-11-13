//! The reference's own cases (tests/libslic3r/test_paint_depth.cpp and the
//! unit cases of test_paint_depth_clamp.cpp), with its numbers.

use edge_geom::{Point2, Polygon};

use super::*;

const LINE: f32 = 0.45;

/// `Flow::rounded_rectangle_extrusion_spacing(0.45, 0.1)`: 0.428540.
fn spacing() -> f32 {
    LINE - 0.1 * (1.0 - 0.25 * std::f64::consts::PI) as f32
}

fn expected(walls: i64, outer_width: f32, outer_spacing: f32, spacing: f32) -> f64 {
    walls as f64 * f64::from(spacing)
        + 2.0 * (f64::from(outer_width) - f64::from(outer_spacing))
        + 0.25 * f64::from(spacing)
}

fn near(a: f64, b: f64, within: f64) -> bool {
    (a - b).abs() <= within
}

#[test]
fn unlimited_is_always_nothing() {
    assert_eq!(band_mm(Mode::Unlimited, 3, 1.5, 0.45, 0.43, 0.42), 0.0);
    assert_eq!(band_mm(Mode::Unlimited, 1, 0.0, 0.0, 0.0, 0.0), 0.0);
}

#[test]
fn millimetres_are_taken_as_set() {
    assert!(near(
        f64::from(band_mm(Mode::Millimeters, 3, 1.5, 0.45, 0.43, 0.42)),
        1.5,
        1e-6
    ));
    assert!(near(
        f64::from(band_mm(Mode::Millimeters, 99, 0.25, 10.0, 10.0, 10.0)),
        0.25,
        1e-6
    ));
    assert!(near(
        f64::from(band_mm(Mode::Millimeters, 1, 2.0, 0.0, 0.0, 0.0)),
        2.0,
        1e-6
    ));
}

#[test]
fn walls_are_spacings_and_arachnes_inset_and_a_quarter_spacing() {
    let s = spacing();
    for walls in [1, 3, 6] {
        let band = band_mm(Mode::Walls, walls, 999.0, LINE, s, s);
        assert!(near(f64::from(band), expected(walls, LINE, s, s), 1e-5));
    }
    // The values the reference was signed off against, at 0.45 mm lines
    // and 0.1 mm layers.
    for (walls, value) in [(1, 0.578595), (3, 1.435675), (6, 2.721294)] {
        assert!(near(
            f64::from(band_mm(Mode::Walls, walls, 0.0, LINE, s, s)),
            value,
            1e-5
        ));
    }
    // Every wall more is one spacing more.
    let bands: Vec<f32> = (2..=4)
        .map(|walls| band_mm(Mode::Walls, walls, 0.0, LINE, s, s))
        .collect();
    assert!(near(f64::from(bands[1] - bands[0]), f64::from(s), 1e-5));
    assert!(near(f64::from(bands[2] - bands[1]), f64::from(s), 1e-5));
    // Under one wall counts as one; no width at all is no band; never
    // under nothing.
    for walls in [0, -5] {
        assert!(near(
            f64::from(band_mm(Mode::Walls, walls, 0.0, LINE, s, s)),
            expected(1, LINE, s, s),
            1e-5
        ));
    }
    assert_eq!(band_mm(Mode::Walls, 3, 0.0, 0.0, 0.0, 0.0), 0.0);
    assert!(band_mm(Mode::Walls, 1, 0.0, 0.1, 0.5, 0.0) >= 0.0);
}

#[test]
fn a_classic_band_is_floored_at_one_wall_stack() {
    let s = spacing();
    let stack = LINE + s;
    assert!(near(f64::from(stack), 0.878540, 1e-5));
    let one = band_mm(Mode::Walls, 1, 0.0, LINE, s, s);
    assert!(near(f64::from(one), 0.578595, 1e-5));
    assert!(near(
        f64::from(band_classic_floor_mm(one, LINE, s)),
        f64::from(stack),
        1e-5
    ));
    for walls in 2..=6 {
        let band = band_mm(Mode::Walls, walls, 0.0, LINE, s, s);
        assert!(band > stack);
        assert!(near(
            f64::from(band_classic_floor_mm(band, LINE, s)),
            f64::from(band),
            1e-6
        ));
    }
    assert!(near(
        f64::from(band_classic_floor_mm(0.4, LINE, s)),
        f64::from(stack),
        1e-5
    ));
    assert!(near(
        f64::from(band_classic_floor_mm(2.0, LINE, s)),
        2.0,
        1e-6
    ));
    assert_eq!(band_classic_floor_mm(0.0, LINE, s), 0.0);
    let unlimited = band_mm(Mode::Unlimited, 3, 1.5, LINE, s, s);
    assert_eq!(band_classic_floor_mm(unlimited, LINE, s), 0.0);
    assert!(near(
        f64::from(band_classic_floor_mm(1.5, 0.0, 0.0)),
        1.5,
        1e-6
    ));
}

#[test]
fn the_notch_is_capped_at_a_quarter_spacing_in_walls_only() {
    let s = spacing();
    let cap = 0.25 * s;
    assert!(near(
        f64::from(interlocking_mm(Mode::Walls, 0.3, s)),
        f64::from(cap),
        1e-6
    ));
    assert!(near(
        f64::from(interlocking_mm(Mode::Walls, 0.1, s)),
        0.1,
        1e-6
    ));
    assert_eq!(interlocking_mm(Mode::Walls, 0.0, s), 0.0);
    assert!(near(
        f64::from(interlocking_mm(Mode::Walls, 0.3, 0.0)),
        0.3,
        1e-6
    ));
    for depth in [0.5, 0.3, 0.1] {
        assert!(near(
            f64::from(interlocking_mm(Mode::Millimeters, depth, s)),
            depth,
            1e-6
        ));
    }
    assert_eq!(interlocking_mm(Mode::Millimeters, 0.0, s), 0.0);
}

#[test]
fn the_notch_never_eats_into_the_walls_asked_for() {
    let s = spacing();
    for walls in 1..=6 {
        for configured in [0.1, 0.3, 1.0] {
            let band = f64::from(band_mm(Mode::Walls, walls, 0.0, LINE, s, s));
            let notch = f64::from(interlocking_mm(Mode::Walls, configured, s));
            let optimum = walls as f64 * f64::from(s) + 2.0 * (f64::from(LINE) - f64::from(s));
            let margin = 0.25 * f64::from(s);
            assert!(band - notch >= optimum - 1e-6);
            assert!(near(band - optimum, margin, 1e-5));
            assert!(band - notch <= optimum + margin + 1e-6);
        }
    }
}

#[test]
fn a_classic_notch_leaves_the_band_at_the_floor_at_least() {
    // At one wall the floored band is the wall stack itself: no notch.
    let s = spacing();
    let stack = LINE + s;
    let band = band_classic_floor_mm(band_mm(Mode::Walls, 1, 0.0, LINE, s, s), LINE, s);
    assert_eq!(classic_notch_cap_mm(0.1, band, stack), 0.0);
    let band = band_mm(Mode::Walls, 3, 0.0, LINE, s, s);
    assert!(near(
        f64::from(classic_notch_cap_mm(0.1, band, stack)),
        0.1,
        1e-6
    ));
    assert_eq!(classic_notch_cap_mm(0.1, 0.0, stack), 0.1);
}

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Vec<ExPolygon> {
    vec![ExPolygon {
        contour: Polygon {
            points: vec![
                Point2 { x: x0, y: y0 },
                Point2 { x: x1, y: y0 },
                Point2 { x: x1, y: y1 },
                Point2 { x: x0, y: y1 },
            ],
        },
        holes: Vec::new(),
    }]
}

#[test]
fn an_island_goes_to_the_colour_it_shares_most_with_the_lowest_on_a_tie() {
    let island = rect(0, 0, 1000, 1000);
    let eps = 10;
    let mut claims = vec![Vec::new(); 4];
    claims[1] = rect(-50, 400, 50, 600);
    claims[2] = rect(500, -500, 1500, 1500);
    assert_eq!(absorb_winner(&island, &claims, eps).unwrap(), 2);

    let mut claims = vec![Vec::new(); 4];
    claims[1] = rect(-500, -500, 500, 1500);
    claims[3] = rect(500, -500, 1500, 1500);
    assert_eq!(absorb_winner(&island, &claims, eps).unwrap(), 1);

    let mut claims = vec![Vec::new(); 4];
    claims[1] = rect(5000, 5000, 6000, 6000);
    assert_eq!(absorb_winner(&island, &claims, eps).unwrap(), 0);
}

#[test]
fn an_island_is_judged_wider_only_beside_a_colour_with_gap_fill_off() {
    let island = rect(0, 0, 1000, 1000);
    let (eps, min_claim) = (10, 0.45f32);
    let mut claims = vec![Vec::new(); 4];
    claims[1] = rect(-500, -500, 0, 1500);
    claims[2] = rect(1000, -500, 1500, 1500);
    claims[3] = rect(3000, 0, 4000, 1000);
    let width =
        |widths: &[f32]| absorb_claim_width(&island, &claims, widths, min_claim, eps).unwrap();
    assert!(near(f64::from(width(&[0.0, 0.0, 0.0, 0.75])), 0.45, 1e-6));
    assert!(near(f64::from(width(&[0.0, 0.0, 0.75, 0.0])), 0.75, 1e-6));
    assert!(near(f64::from(width(&[0.0, 0.60, 0.75, 0.0])), 0.75, 1e-6));
    assert!(near(f64::from(width(&[])), 0.45, 1e-6));
}

#[test]
fn a_part_thinner_than_twice_the_band_keeps_half_of_itself() {
    // A 40 mm wide part keeps all but the band; a 3 mm fin, under twice a
    // 2 mm band, keeps a core from the ladder's first step: shrunk by a
    // quarter band, 0.5 mm, from each side.
    let scale = |mm: f64| (mm * 1e6) as i64;
    let band = scaled_f32(2.0);
    let wide = rect(0, 0, scale(40.0), scale(40.0));
    let core = keep_core(&wide, band, band, scaled_f32(0.45)).unwrap();
    assert!(near(area(&core), 36.0 * 36.0, 1e-6));
    let fin = rect(0, 0, scale(3.0), scale(40.0));
    let core = keep_core(&fin, band, band, scaled_f32(0.45)).unwrap();
    assert!(near(area(&core), 2.0 * 39.0, 1e-6), "{}", area(&core));
    // With the ladder's first step under the narrowest claim nothing is
    // kept: the fin is all paint, as without a bound.
    let core = keep_core(&fin, band, band, scaled_f32(0.6)).unwrap();
    assert!(core.is_empty());
}
