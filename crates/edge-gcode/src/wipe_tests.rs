//! The moves round a wall's seam: where they go for each way a wall can
//! be laid, and what the writer makes of them.

// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use std::f64::consts::PI;
use std::ops::Range;

use edge_config::{FloatOrPercent, Settings};
use edge_geom::Polygon;

use super::*;
use crate::{Layer, Lift, PrintSettings, Seam, Summary, WallOrder, write};

const S: i64 = 1_000_000;

fn p(x: i64, y: i64) -> Point2 {
    Point2 { x, y }
}

fn flow(width: f64) -> Flow {
    Flow::new(width, 0.2, 0.4).unwrap()
}

/// A square wall with its lower-left corner at `(x, y)`.
fn wall(x: f64, y: f64, side: f64, depth: usize) -> ExtrusionLoop {
    let points = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
        .map(|(x, y)| Point2::from_mm(x, y));
    ExtrusionLoop {
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
        polygon: Polygon {
            points: points.to_vec(),
        },
        role: if depth == 0 {
            Role::ExternalPerimeter
        } else {
            Role::Perimeter
        },
        is_hole: false,
        depth,
        flow: flow(if depth == 0 { 0.42 } else { 0.45 }),
    }
}

/// A part 20 mm square with two walls, and another 40 mm off, so the
/// nozzle has somewhere far to go when the first is done.
fn two_parts() -> [ExtrusionLoop; 4] {
    [
        wall(0.0, 0.0, 20.0, 0),
        wall(0.435, 0.435, 19.13, 1),
        wall(60.0, 0.0, 20.0, 0),
        wall(60.435, 0.435, 19.13, 1),
    ]
}

fn basic() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        lift: Lift::Normal,
        slow_down_for_layer_cooling: false,
        ..PrintSettings::default()
    }
}

fn print(
    loops: &[ExtrusionLoop],
    islands: &[Range<usize>],
    settings: &PrintSettings,
) -> (String, Summary) {
    let layers = [Layer {
        region: None,
        z: 0.2,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops,
        islands,
        paths: &[],
        shelter: &[],
        below: &[],
    }];
    let mut out = Vec::new();
    let summary = write(&mut out, &layers, settings).unwrap();
    (String::from_utf8(out).unwrap(), summary)
}

/// The value of `word` (such as `E`) on a G-code line, if it has one.
fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

/// The lines from the first one that starts with `start`, `count` of them.
fn from<'a>(text: &'a str, start: &str, count: usize) -> Vec<&'a str> {
    text.lines()
        .skip_while(|line| !line.starts_with(start))
        .take(count)
        .collect()
}

/// The layer's own lines: what follows the first `;TYPE:`.
fn body(text: &str) -> Vec<&str> {
    text.lines()
        .skip_while(|line| !line.starts_with(";TYPE:"))
        .take_while(|line| !line.starts_with("; filament used"))
        .collect()
}

/// What the layer feeds forward along its lines.
fn fed(text: &str) -> f64 {
    body(text)
        .iter()
        .filter(|line| line.starts_with("G1 X"))
        .filter_map(|line| word(line, 'E'))
        .filter(|&e| e > 0.0)
        .sum()
}

// GCode/WipePathHelpers.cpp's own tests of `sample_path_at_distance`
// (tests/libslic3r/test_wipe_path.cpp).
#[test]
fn a_path_is_sampled_from_either_end() {
    let path = [p(0, 0), p(100 * S, 0), p(100 * S, 100 * S)];
    assert_eq!(sample_path_at_distance(&path, true, 0.0), p(0, 0));
    assert_eq!(sample_path_at_distance(&path, true, 50e6), p(50 * S, 0));
    assert_eq!(
        sample_path_at_distance(&path, true, 150e6),
        p(100 * S, 50 * S)
    );
    assert_eq!(
        sample_path_at_distance(&path, false, 50e6),
        p(100 * S, 50 * S)
    );
    // A path too short gives what can be reached, and a point itself.
    assert_eq!(
        sample_path_at_distance(&[p(0, 0), p(10 * S, 0)], true, 1000e6),
        p(10 * S, 0)
    );
    let point = [p(50 * S, 50 * S)];
    assert_eq!(sample_path_at_distance(&point, true, 100e6), point[0]);
    assert_eq!(sample_path_at_distance(&point, false, 100e6), point[0]);
}

// "wipe_on_loops destination is on the material side for every
// orientation" (test_wipe_path.cpp).
#[test]
fn a_loop_ends_with_a_move_to_the_side_the_part_is_on() {
    for (is_ccw, is_hole) in [(true, false), (false, false), (false, true), (true, true)] {
        for nozzle in [0.4, 0.8] {
            for subdivided in [false, true] {
                let mut contour = if subdivided {
                    // "The same square, with path boundaries inside both
                    // sampling distances near the seam."
                    vec![
                        p(0, 0),
                        p(30_000, 0),
                        p(200_000, 0),
                        p(20 * S, 0),
                        p(20 * S, 20 * S),
                        p(0, 20 * S),
                        p(0, 200_000),
                        p(0, 30_000),
                    ]
                } else {
                    vec![p(0, 0), p(20 * S, 0), p(20 * S, 20 * S), p(0, 20 * S)]
                };
                if !is_ccw {
                    for point in &mut contour {
                        std::mem::swap(&mut point.x, &mut point.y);
                    }
                }
                contour.push(contour[0]);
                let destination =
                    on_loops_destination(&contour, nozzle * 1e6, is_ccw, is_hole).unwrap();
                let seam = contour[0];
                let first_edge = V::between(seam, contour[1]).normalized();
                let mut material = V {
                    x: -first_edge.y,
                    y: first_edge.x,
                };
                if is_ccw == is_hole {
                    material = material.times(-1.0);
                }
                let moved = V::between(seam, destination);
                assert!(moved.dot(material) > 0.0);
                // "Move 20% of the nozzle diameter, turning through one
                // third of the material-side corner: 90 degrees for a
                // contour, 270 degrees for a hole."
                let distance = 0.2 * nozzle * 1e6;
                let angle = if is_hole { PI / 2.0 } else { PI / 6.0 };
                assert!((moved.dot(first_edge) - distance * angle.cos()).abs() <= 2.0);
                assert!((moved.dot(material) - distance * angle.sin()).abs() <= 2.0);
            }
        }
    }
    // "a 0.5 mm square is tight for a 0.4 mm nozzle but remains valid."
    let small = [p(0, 0), p(S / 2, 0), p(S / 2, S / 2), p(0, S / 2), p(0, 0)];
    assert!(on_loops_destination(&small, 0.4e6, true, false).is_some());
    // No loop at all gives nowhere to go.
    assert!(on_loops_destination(&[p(50 * S, 50 * S)], 0.4e6, true, false).is_none());
    // Nor does one that doubles back on itself at its seam.
    let spike = [p(0, 0), p(10 * S, 0), p(0, 0)];
    assert!(on_loops_destination(&spike, 0.4e6, true, false).is_none());
}

#[test]
fn an_outer_wall_is_come_at_from_the_side_the_part_is_on() {
    // A square 20 mm a side, opened at its corner and stopped 0.06 mm
    // short of it, or not short at all.
    for gap in [60_000, 0] {
        for (is_ccw, is_hole) in [(true, false), (false, false), (false, true), (true, true)] {
            let mut path = vec![
                p(0, 0),
                p(20 * S, 0),
                p(20 * S, 20 * S),
                p(0, 20 * S),
                p(0, gap),
            ];
            if !is_ccw {
                for point in &mut path {
                    std::mem::swap(&mut point.x, &mut point.y);
                }
            }
            let point = approach_point(&path, 0.2e6, is_ccw, is_hole).unwrap();
            let first_edge = V::between(path[0], path[1]).normalized();
            let mut material = V {
                x: -first_edge.y,
                y: first_edge.x,
            };
            if is_ccw == is_hole {
                material = material.times(-1.0);
            }
            // Twice the reach from the wall's start, a third of the
            // corner round from the wall's last edge toward its first,
            // on the side the part is on: 30 degrees for a part, which
            // is in behind the first edge, and 90 for a hole, whose
            // corner is 270 degrees on that side, which is straight on
            // past the corner.
            let to = V::between(path[0], point);
            let (along, across) = if is_hole {
                (-0.4e6, 0.0)
            } else {
                (0.4e6 * 0.5, 0.4e6 * 0.75f64.sqrt())
            };
            assert!(
                (to.dot(first_edge) - along).abs() <= 2.0,
                "{is_ccw} {is_hole} {to:?}"
            );
            assert!((to.dot(material) - across).abs() <= 2.0, "{to:?}");
        }
    }
    // A loop with nothing to it gives nowhere to come from.
    assert!(approach_point(&[p(0, 0), p(0, 0)], 0.2e6, true, false).is_none());
    assert!(approach_point(&[p(0, 0)], 0.2e6, true, false).is_none());
}

#[test]
fn the_settings_are_read_and_none_is_left_as_a_note() {
    let read = |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap());
    let defaults = read("{}").unwrap();
    assert!(!defaults.wipe_on_loops && !defaults.wipe_before_external_loop);
    assert_eq!((defaults.wipe_inward, defaults.wall_loops), (None, 2));
    let on = read(
        r#"{"wipe_on_loops": "1", "wipe_before_external_loop": "1", "wipe_inward": "1",
            "wall_loops": "3"}"#,
    )
    .unwrap();
    assert!(on.wipe_on_loops && on.wipe_before_external_loop);
    // PrintConfig.cpp: half the outer wall's width unless set.
    assert_eq!(on.wipe_inward, Some(FloatOrPercent::Percent(50.0)));
    assert_eq!(on.wall_loops, 3);
    assert!(
        !on.unsupported.iter().any(|note| note.contains("wipe")),
        "{:?}",
        on.unsupported
    );
    let set = read(r#"{"wipe_inward": "1", "wipe_inward_distance": "0.3"}"#).unwrap();
    assert_eq!(set.wipe_inward, Some(FloatOrPercent::Value(0.3)));
    // The distance alone asks for nothing.
    let off = read(r#"{"wipe_inward": "0", "wipe_inward_distance": "80%"}"#).unwrap();
    assert_eq!(off.wipe_inward, None);
    assert!(read(r#"{"wipe_on_loops": "maybe"}"#).is_err());
}

#[test]
fn an_outer_wall_ends_with_a_move_in_before_the_nozzle_leaves() {
    let loops = two_parts();
    let islands = [0..2, 2..4];
    let plain = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        ..basic()
    };
    let asked = PrintSettings {
        wipe_on_loops: true,
        ..plain.clone()
    };
    let (without, _) = print(&loops, &islands, &plain);
    let (with, summary) = print(&loops, &islands, &asked);
    // The outer wall starts at (0, 0), goes round by (20, 0) and stops
    // 0.04 mm short. The move in ends 0.08 mm from the start, a third of
    // the corner round from the first edge, and says no speed and feeds
    // nothing; the wipe sets off from there, back onto the wall.
    let end = from(&with, "G1 X0.000 Y0.040 E", 6);
    assert_eq!(end[1], "G1 X0.069 Y0.040", "{with}");
    assert!(end[2].starts_with("G1 E-"), "{end:?}");
    assert_eq!(end[3], ";WIPE_START");
    let wiped = (word(end[5], 'X').unwrap(), word(end[5], 'Y').unwrap());
    assert!((wiped.0 - 2.069).abs() < 0.002 && wiped.1 < 0.04, "{end:?}");
    // One such move for each outer wall, and none for the inner walls:
    // a move that feeds nothing straight after one that does.
    let dry = |text: &str| {
        body(text)
            .windows(2)
            .filter(|pair| {
                pair[0].starts_with("G1 X")
                    && word(pair[0], 'E').is_some()
                    && pair[1].starts_with("G1 X")
                    && !pair[1].contains(['E', 'F', 'Z'])
            })
            .count()
    };
    assert_eq!((dry(&without), dry(&with)), (0, 2));
    // Nothing more is laid for it, and the two moves are counted as travel.
    assert!((fed(&with) - fed(&without)).abs() < 1e-9);
    let (_, plain_summary) = print(&loops, &islands, &plain);
    assert_eq!(summary.filament_mm, plain_summary.filament_mm);
    assert!(summary.travel_mm > plain_summary.travel_mm);

    // A part set to have one wall gets none ("m_config.wall_loops.value > 1").
    let single = PrintSettings {
        wall_loops: 1,
        ..asked.clone()
    };
    assert_eq!(dry(&print(&loops, &islands, &single).0), 0);
    // Nor is a setting that is off any different from one never heard of.
    assert_eq!(
        without,
        print(
            &loops,
            &islands,
            &PrintSettings {
                wall_loops: 5,
                ..plain
            }
        )
        .0
    );
}

#[test]
fn an_outer_wall_is_come_at_from_inside_the_part() {
    let loops = two_parts();
    let islands = [0..2, 2..4];
    let plain = PrintSettings {
        wall_order: WallOrder::OuterInner,
        initial_layer_speed: 50.0,
        ..basic()
    };
    let asked = PrintSettings {
        wipe_before_external_loop: true,
        ..plain.clone()
    };
    let (without, plain_summary) = print(&loops, &islands, &plain);
    let (with, summary) = print(&loops, &islands, &asked);
    // The second part's outer wall starts at (60, 0), the nearest corner
    // to where the first part ended, after a retraction. The travel ends
    // 0.4 mm from there, inside the part; the filament comes forward
    // there; and the nozzle moves out to the wall's start at the outer
    // wall's speed, laying nothing.
    let second = with
        .lines()
        .skip_while(|line| !line.starts_with("G1 E-"))
        .collect::<Vec<_>>();
    let at = second
        .iter()
        .position(|line| line.starts_with("G1 X60.200 Y0.346"))
        .unwrap_or_else(|| panic!("{with}"));
    let after: Vec<&str> = second[at + 1..]
        .iter()
        .copied()
        .filter(|line| line.starts_with("G1"))
        .take(4)
        .collect();
    assert!(after[0].starts_with("G1 Z0.200"), "{after:?}");
    assert!(after[1].starts_with("G1 E0.8"), "{after:?}");
    assert_eq!(after[2], "G1 X60.000 Y0.000 F3000", "{after:?}");
    assert!(after[3].starts_with("G1 X80.000 Y0.000 E"), "{after:?}");
    // Nothing more is laid, and nothing is fed on the way out.
    assert!((fed(&with) - fed(&without)).abs() < 1e-9);
    assert_eq!(summary.filament_mm, plain_summary.filament_mm);
    assert_eq!(summary.retractions, plain_summary.retractions);

    // A part with one wall has nothing inside its outer wall to come
    // from ("we will extrude in mid air as no neighbouring perimeter
    // exists").
    let lone = [wall(0.0, 0.0, 20.0, 0), wall(60.0, 0.0, 20.0, 0)];
    let lone_islands = [0..1, 1..2];
    assert_eq!(
        print(&lone, &lone_islands, &asked).0,
        print(&lone, &lone_islands, &plain).0
    );
    // Nor has one whose second wall is nowhere near the first.
    let apart = [
        wall(0.0, 0.0, 20.0, 0),
        wall(5.0, 5.0, 10.0, 1),
        wall(60.0, 0.0, 20.0, 0),
        wall(65.0, 5.0, 10.0, 1),
    ];
    assert_eq!(
        print(&apart, &islands, &asked).0,
        print(&apart, &islands, &plain).0
    );
}

#[test]
fn the_nozzle_is_wiped_over_the_inner_wall_behind_an_outer_one() {
    let loops = two_parts();
    let islands = [0..2, 2..4];
    let plain = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        retract_before_wipe: 0.0,
        ..basic()
    };
    let asked = PrintSettings {
        wipe_inward: Some(FloatOrPercent::Percent(50.0)),
        ..plain.clone()
    };
    let (without, plain_summary) = print(&loops, &islands, &plain);
    let (with, summary) = print(&loops, &islands, &asked);
    // The inner wall goes down first. The outer wall then ends 0.04 mm
    // short of (0, 0), and the wipe goes half the wall's width in from
    // it and on beside the wall's first edge, pulling nothing back; the
    // whole retraction follows where the wipe ends.
    let end = from(&with, "G1 X0.000 Y0.040 E", 7);
    assert_eq!(
        end[1..],
        [
            ";WIPE_START",
            "G1 F1800",
            "G1 X0.210 Y0.210",
            "G1 X1.940 Y0.210",
            ";WIPE_END",
            "G1 E-0.80000"
        ],
        "{with}"
    );
    // As long a wipe as was asked for, from where the nozzle stopped.
    let first = (0.21f64).hypot(0.17);
    assert!((first + 1.94 - 0.21 - 2.0).abs() < 0.002);
    assert!((fed(&with) - fed(&without)).abs() < 1e-9);
    assert_eq!(summary.filament_mm, plain_summary.filament_mm);
    // In millimetres it is taken as that far.
    let nearer = PrintSettings {
        wipe_inward: Some(FloatOrPercent::Value(0.1)),
        ..plain.clone()
    };
    assert_eq!(
        from(&print(&loops, &islands, &nearer).0, ";WIPE_START", 3)[2],
        "G1 X0.100 Y0.100"
    );
    // And never further in than the wall is wide.
    let further = PrintSettings {
        wipe_inward: Some(FloatOrPercent::Percent(300.0)),
        ..plain.clone()
    };
    assert_eq!(
        from(&print(&loops, &islands, &further).0, ";WIPE_START", 3)[2],
        "G1 X0.420 Y0.420"
    );

    // With the outer wall first there is no inner wall down to wipe
    // over, and the nozzle is wiped along the wall as ever; so it is
    // with no wipe asked for at all, or one of no length.
    let outer_first = |settings: &PrintSettings| PrintSettings {
        wall_order: WallOrder::OuterInner,
        ..settings.clone()
    };
    assert_eq!(
        print(&loops, &islands, &outer_first(&asked)).0,
        print(&loops, &islands, &outer_first(&plain)).0
    );
    for (wipe, wipe_distance) in [(false, 2.0), (true, 0.0)] {
        let quiet = |settings: &PrintSettings| PrintSettings {
            wipe,
            wipe_distance,
            ..settings.clone()
        };
        assert_eq!(
            print(&loops, &islands, &quiet(&asked)).0,
            print(&loops, &islands, &quiet(&plain)).0
        );
    }
    // A distance of nothing is the option off.
    let none = PrintSettings {
        wipe_inward: Some(FloatOrPercent::Percent(0.0)),
        ..plain.clone()
    };
    assert_eq!(print(&loops, &islands, &none).0, without);
}

#[test]
fn the_three_together_keep_to_the_bed_and_lay_nothing() {
    let loops = two_parts();
    let islands = [0..2, 2..4];
    let plain = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        retract_before_wipe: 0.7,
        // The bed ends where the outer walls' lines do.
        bed: Some([0.0, 0.0, 80.0, 20.0]),
        ..basic()
    };
    for wall_order in [
        WallOrder::InnerOuter,
        WallOrder::OuterInner,
        WallOrder::InnerOuterInner,
    ] {
        let plain = PrintSettings {
            wall_order,
            ..plain.clone()
        };
        let asked = PrintSettings {
            wipe_on_loops: true,
            wipe_before_external_loop: true,
            wipe_inward: Some(FloatOrPercent::Percent(50.0)),
            ..plain.clone()
        };
        let (without, plain_summary) = print(&loops, &islands, &plain);
        let (with, summary) = print(&loops, &islands, &asked);
        assert_ne!(with, without);
        assert!((fed(&with) - fed(&without)).abs() < 1e-9);
        assert_eq!(summary.filament_mm, plain_summary.filament_mm);
        for line in body(&with) {
            if let (Some(x), Some(y)) = (word(line, 'X'), word(line, 'Y')) {
                assert!(
                    (0.0..=80.0).contains(&x) && (0.0..=20.0).contains(&y),
                    "{line}"
                );
            }
        }
        // The same twice over.
        assert_eq!(with, print(&loops, &islands, &asked).0);
    }
}

#[test]
fn a_move_that_would_leave_the_bed_is_not_made() {
    // A wall laid clockwise that is said to go round a hole has its
    // part outside it, and with the bed ending at the wall's line the
    // moves toward the part would end off it.
    let mut loops = two_parts();
    for wall in &mut loops {
        wall.is_hole = true;
        wall.polygon.points.reverse();
    }
    let islands = [0..2, 2..4];
    let plain = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        wall_order: WallOrder::OuterInner,
        ..basic()
    };
    let asked = PrintSettings {
        wipe_on_loops: true,
        wipe_before_external_loop: true,
        ..plain.clone()
    };
    let held = |settings: &PrintSettings| PrintSettings {
        bed: Some([0.0, 0.0, 80.0, 20.0]),
        ..settings.clone()
    };
    // With room they are made, outward as it is.
    assert_ne!(
        print(&loops, &islands, &asked).0,
        print(&loops, &islands, &plain).0
    );
    let (with, _) = print(&loops, &islands, &held(&asked));
    for line in body(&with) {
        if let (Some(x), Some(y)) = (word(line, 'X'), word(line, 'Y')) {
            assert!(
                (0.0..=80.0).contains(&x) && (0.0..=20.0).contains(&y),
                "{line}"
            );
        }
    }
}

#[test]
fn a_square_holes_wall_is_come_at_from_past_its_corner() {
    // The wall round a square hole, and the wall behind it, which is the
    // larger. On the side the part is on a hole's corner is 270
    // degrees, so the point the wall is come at from is a third of that
    // round from its last edge: straight on past the corner, exactly as
    // far from it as the nozzle is wide. The wall behind is beside that
    // point, and the wall is come at from it. (In the reference this
    // turns on a rounding: it wants the hole's own wall nearer the point
    // than the nozzle's width, which is just what it is not.)
    let hole = |x: f64, y: f64, side: f64, depth: usize| {
        let mut wall = wall(x, y, side, depth);
        wall.is_hole = true;
        wall.polygon.points.reverse();
        wall
    };
    let loops = [hole(10.0, 10.0, 20.0, 0), hole(9.565, 9.565, 20.87, 1)];
    let asked = PrintSettings {
        wipe_before_external_loop: true,
        wall_order: WallOrder::OuterInner,
        initial_layer_speed: 50.0,
        ..basic()
    };
    let (text, _) = print(&loops, &[0..2], &asked);
    let lines = body(&text);
    let first = lines
        .iter()
        .position(|line| line.starts_with("G1 X") && word(line, 'E').is_some())
        .unwrap();
    // The move that lays nothing ends where the wall starts, 0.4 mm
    // from where the travel before it ended, and in line with the
    // wall's first edge.
    let before: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("G1 X"))
        .take(3)
        .collect();
    let at = |line: &str| (word(line, 'X').unwrap(), word(line, 'Y').unwrap());
    let (from, start, next) = (at(before[0]), at(before[1]), at(before[2]));
    assert!(
        word(before[1], 'E').is_none() && word(before[2], 'E').is_some(),
        "{before:?}"
    );
    assert_eq!(before[2], lines[first]);
    let reach = (start.0 - from.0).hypot(start.1 - from.1);
    assert!((reach - 0.4).abs() < 1e-3, "{before:?}");
    let cross = (start.0 - from.0) * (next.1 - start.1) - (start.1 - from.1) * (next.0 - start.0);
    assert!(cross.abs() < 1e-6, "{before:?}");
}
