//! Lines contoured in height, as the writer lays them (`contour.rs`).

// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use std::sync::Arc;

use edge_contour::{LayerProfiles, Profile};

use super::*;

fn mm(x: f64, y: f64) -> Point2 {
    Point2::from_mm(x, y)
}

fn settings() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        seam_gap: 0.0,
        lift: Lift::Normal,
        z_hop: 0.0,
        wipe: true,
        wipe_distance: 1.0,
        retraction_length: 0.8,
        retraction_minimum_travel: 0.5,
        slow_down_for_layer_cooling: false,
        outer_wall_speed: 60.0,
        inner_wall_speed: 80.0,
        top_surface_speed: 100.0,
        initial_layer_speed: 20.0,
        filament_max_volumetric_speed: 0.0,
        enable_overhang_speed: false,
        detect_overhang_wall: false,
        overhang_fan: None,
        small_perimeter_threshold: 0.0,
        ..PrintSettings::default()
    }
}

/// A top surface line 4 mm along x at y = 0, its heights from 0.1 mm
/// below the layer's top at x = 0 up to it at x = 4, in points 0.5 mm
/// apart; and a wall square round it, 0.05 mm below the top all round.
fn contoured() -> (ExtrusionPath, Arc<Profile>, ExtrusionLoop, Arc<Profile>) {
    let points: Vec<Point2> = (0..=8).map(|i| mm(0.5 * i as f64, 0.0)).collect();
    let heights: Vec<f32> = (0..=8).map(|i| (-0.1 + 0.0125 * i as f64) as f32).collect();
    let path = ExtrusionPath {
        points: points.clone(),
        role: Role::TopSolidInfill,
        flow: Flow::new(0.42, 0.2, 0.4).unwrap(),
        in_turn: false,
        part: 1,
    };
    let corners = vec![mm(-2.0, -2.0), mm(6.0, -2.0), mm(6.0, 2.0), mm(-2.0, 2.0)];
    let mut round = corners.clone();
    round.push(corners[0]);
    let wall = ExtrusionLoop {
        polygon: edge_geom::Polygon { points: corners },
        role: Role::ExternalPerimeter,
        is_hole: false,
        depth: 0,
        flow: Flow::new(0.45, 0.2, 0.4).unwrap(),
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
    };
    let wall_heights = vec![-0.05; round.len()];
    (
        path,
        Arc::new(Profile::new(points, heights)),
        wall,
        Arc::new(Profile::new(round, wall_heights)),
    )
}

/// Two layers, the second's line and wall contoured.
fn written(settings: &PrintSettings, speed_scaling: bool) -> String {
    let (path, heights, wall, wall_heights) = contoured();
    let paths = [path];
    let loops = [wall];
    let islands = [0..1];
    let layer = |z: f64| Layer {
        region: None,
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        wipe: None,
        tower: &[],
        around: &[],
        support: &[],
        loops: &loops,
        islands: &islands,
        paths: &paths,
        shelter: &[],
        below: &[],
    };
    let flat = LayerProfiles::default();
    let profiles = LayerProfiles {
        loops: vec![Some(wall_heights)],
        paths: vec![Some(heights)],
    };
    let mut out = Vec::new();
    write_contoured(
        &mut out,
        &[layer(0.2), layer(0.4)],
        &[],
        &SeamScene::default(),
        &Contours {
            layers: &[&flat, &profiles],
            speed_scaling,
        },
        std::slice::from_ref(settings),
    )
    .unwrap();
    String::from_utf8(out).unwrap()
}

fn word(line: &str, word: char) -> Option<f64> {
    line.split(';')
        .next()?
        .split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

/// The moves of the layer at 0.4 mm of one feature: (x, y, z said, E, F in force).
fn moves(text: &str, feature: &str) -> Vec<(f64, f64, Option<f64>, f64, f64)> {
    let mut on = false;
    let mut kind = "";
    let mut feed = 0.0;
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(z) = line.strip_prefix(";Z:") {
            on = z == "0.400";
        }
        if let Some(name) = line.strip_prefix(";TYPE:") {
            kind = if name == feature { feature } else { "" };
        }
        if let Some(f) = word(line, 'F') {
            feed = f;
        }
        if on
            && kind == feature
            && line.starts_with("G1 X")
            && let Some(e) = word(line, 'E').filter(|e| *e > 0.0)
        {
            out.push((
                word(line, 'X').unwrap(),
                word(line, 'Y').unwrap(),
                word(line, 'Z'),
                e,
                feed,
            ));
        }
    }
    out
}

#[test]
fn every_move_of_a_contoured_line_says_its_height_and_lays_as_much_as_it_is_tall() {
    let text = written(&settings(), false);
    let top = moves(&text, "Top surface");
    assert_eq!(top.len(), 8, "{text}");
    let flow = Flow::new(0.42, 0.2, 0.4).unwrap();
    let area = PI * (1.75f64 / 2.0).powi(2);
    let mut x0: Option<f64> = None;
    for &(x, _, z, e, _) in &top {
        // The line may be laid either way: its height goes by where it is.
        let wanted = 0.4 - 0.1 + 0.0125 * x / 0.5;
        assert!((z.unwrap() - wanted).abs() < 6e-4, "{x}: {z:?}");
        if let Some(from) = x0 {
            let mean = 0.5 * ((-0.1 + 0.025 * from) + (-0.1 + 0.025 * x));
            let per_mm = flow.mm3_per_mm() * (0.2 + mean) / 0.2 / area;
            assert!((e / 0.5 - per_mm).abs() < 2e-4, "{x}: {e}");
        }
        x0 = Some(x);
    }
    // The wall all round 0.05 below the top, and no scarf or arc.
    let wall = moves(&text, "Outer wall");
    assert!(!wall.is_empty());
    assert!(
        wall.iter().all(|&(_, _, z, ..)| z == Some(0.35)),
        "{wall:?}"
    );
    assert!(!text.contains("\nG2 ") && !text.contains("\nG3 "));
    // The first layer is flat.
    let first = text.split(";Z:0.400").next().unwrap();
    assert!(!first.lines().any(|line| line.starts_with("G1 X")
        && line.contains(" Z")
        && line.contains(" E")
        && !line.contains("E-")));
}

#[test]
fn a_thinner_move_goes_slower_with_speed_scaling() {
    let text = written(&settings(), true);
    let top = moves(&text, "Top surface");
    // 100 mm/s at the layer's height; at 0.1 mm below it on a 0.2 mm
    // layer about half that, coming up in steps of 5 % as the line does.
    let slowest = top.iter().map(|m| m.4).fold(f64::MAX, f64::min);
    let fastest = top.iter().map(|m| m.4).fold(0.0, f64::max);
    assert!(
        slowest < 0.6 * 6000.0 && slowest > 0.5 * 6000.0,
        "{slowest}"
    );
    assert!(fastest <= 6000.0 && fastest > 0.9 * 6000.0, "{fastest}");
    // Flat, the line goes at its own speed.
    let plain = moves(&written(&settings(), false), "Top surface");
    assert!(plain.iter().all(|m| m.4 == 6000.0), "{plain:?}");
}

#[test]
fn a_wipe_over_a_contoured_line_keeps_to_its_heights() {
    let text = written(&settings(), false);
    let mut in_wipe = false;
    let mut seen = 0;
    // From the second layer's first line on: the wipe at the change of
    // layer is the first layer's, flat.
    let mut layer = "";
    for line in text.lines() {
        if line.strip_prefix(";Z:") == Some("0.400") {
            layer = "changed";
        }
        if layer == "changed"
            && line.starts_with("G1 X")
            && word(line, 'E').is_some_and(|e| e > 0.0)
        {
            layer = "second";
        }
        match line {
            ";WIPE_START" => in_wipe = true,
            ";WIPE_END" => in_wipe = false,
            _ if in_wipe && layer == "second" && line.starts_with("G1 X") => {
                let (x, y) = (word(line, 'X').unwrap(), word(line, 'Y').unwrap());
                let z = word(line, 'Z').unwrap_or_else(|| {
                    panic!("a wipe over a contoured line says its height: {line}\n{text}")
                });
                if y.abs() < 1e-6 && (0.0..=4.0).contains(&x) {
                    assert!((z - (0.3 + 0.025 * x)).abs() < 6e-4, "{line}");
                } else {
                    assert!((z - 0.35).abs() < 6e-4, "{line}");
                }
                seen += 1;
            }
            _ => {}
        }
    }
    assert!(seen > 0, "{text}");
}

#[test]
fn without_contours_the_writer_lays_what_it_always_has() {
    let (path, _, wall, _) = contoured();
    let paths = [path];
    let loops = [wall];
    let islands = [0..1];
    let layer = |z: f64| Layer {
        region: None,
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        wipe: None,
        tower: &[],
        around: &[],
        support: &[],
        loops: &loops,
        islands: &islands,
        paths: &paths,
        shelter: &[],
        below: &[],
    };
    let settings = settings();
    let mut plain = Vec::new();
    write(&mut plain, &[layer(0.2), layer(0.4)], &settings).unwrap();
    let flat = LayerProfiles::default();
    let mut empty = Vec::new();
    write_contoured(
        &mut empty,
        &[layer(0.2), layer(0.4)],
        &[],
        &SeamScene::default(),
        &Contours {
            layers: &[&flat, &flat],
            speed_scaling: true,
        },
        std::slice::from_ref(&settings),
    )
    .unwrap();
    assert_eq!(plain, empty);
}
