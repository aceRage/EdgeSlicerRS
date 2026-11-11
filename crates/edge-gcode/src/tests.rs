// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use edge_geom::Polygon;

use super::*;

fn flow() -> Flow {
    Flow::new(0.45, 0.2, 0.4).unwrap()
}

/// A square wall with its lower-left corner at `(x, y)`.
fn wall(x: f64, y: f64, side: f64, depth: usize) -> ExtrusionLoop {
    let points = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
        .map(|(x, y)| Point2::from_mm(x, y));
    let role = if depth == 0 {
        Role::ExternalPerimeter
    } else {
        Role::Perimeter
    };
    ExtrusionLoop {
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
        polygon: Polygon {
            points: points.to_vec(),
        },
        role,
        is_hole: false,
        depth,
        flow: flow(),
    }
}

fn line(a: (f64, f64), b: (f64, f64), role: Role) -> ExtrusionPath {
    ExtrusionPath {
        points: vec![Point2::from_mm(a.0, a.1), Point2::from_mm(b.0, b.1)],
        role,
        flow: flow(),
        in_turn: false,
        part: 0,
    }
}

/// The reference's defaults, less what would get in the way of reading a
/// small test: walls start at the nearest corner and close fully, lifts go
/// straight up, and layers that take no time at all are not slowed down.
fn basic() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        seam_gap: 0.0,
        lift: Lift::Normal,
        slow_down_for_layer_cooling: false,
        ..PrintSettings::default()
    }
}

fn print(layers: &[Layer<'_>], settings: &PrintSettings) -> (String, Summary) {
    let mut out = Vec::new();
    let summary = write(&mut out, layers, settings).unwrap();
    (String::from_utf8(out).unwrap(), summary)
}

/// One layer at z = 0.2 holding these walls (each island a range) and fill.
fn one_layer<'a>(
    loops: &'a [ExtrusionLoop],
    islands: &'a [Range<usize>],
    paths: &'a [ExtrusionPath],
) -> [Layer<'a>; 1] {
    [Layer {
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
        paths,
        shelter: &[],
        below: &[],
    }]
}

/// The value of `word` (such as `E`) on a G-code line, if it has one.
fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

fn moves(text: &str) -> impl Iterator<Item = &str> {
    text.lines().filter(|line| line.starts_with("G1 "))
}

fn features(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.strip_prefix(";TYPE:"))
        .collect()
}

#[test]
fn filament_fed_matches_the_plastic_in_the_lines() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [
        line((2.0, 2.0), (8.0, 2.0), Role::SparseInfill),
        line((2.0, 4.0), (8.0, 4.0), Role::SparseInfill),
    ];
    let settings = PrintSettings {
        filament_flow_ratio: 0.98,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&loops, &[0..1], &fill), &settings);

    let volume = (40.0 + 12.0) * flow().mm3_per_mm() * 0.98;
    assert!((summary.filament_mm3 - volume).abs() < 1e-9);
    assert!((summary.filament_mm - volume / (PI * 0.875 * 0.875)).abs() < 1e-9);
    // What the file says agrees with the summary to its printed precision.
    let fed: f64 = moves(&text)
        .filter(|line| line.contains(" X"))
        .filter_map(|line| word(line, 'E'))
        .sum();
    assert!(
        (fed - summary.filament_mm).abs() < 1e-4,
        "file feeds {fed}, summary says {}",
        summary.filament_mm
    );
    assert!(text.contains("M83") && text.contains(";LAYER_CHANGE\n;Z:0.200\n;HEIGHT:0.200"));
    assert!(text.contains("; filament used [mm] = "));
}

#[test]
fn a_wall_goes_once_round_and_closes() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let (text, _) = print(&one_layer(&loops, &[0..1], &[]), &basic());
    let extruding: Vec<&str> = moves(&text)
        .filter(|line| line.contains(" E") && line.contains(" X"))
        .collect();
    assert_eq!(extruding.len(), 4);
    assert!(
        extruding[3].starts_with("G1 X0.000 Y0.000 E"),
        "last move was {}",
        extruding[3]
    );
    assert_eq!(features(&text), ["Outer wall"]);
    assert!(text.contains(";WIDTH:0.450"));
}

#[test]
fn long_travels_retract_and_lift_and_short_ones_do_not() {
    let settings = basic();
    let far = [wall(0.0, 0.0, 10.0, 0), wall(50.0, 0.0, 10.0, 0)];
    let (text, summary) = print(&one_layer(&far, &[0..1, 1..2], &[]), &settings);
    // One retraction between the islands and one at the end of the print.
    assert_eq!(summary.retractions, 2);
    let after_retract: Vec<&str> = text
        .lines()
        .skip_while(|line| !line.starts_with("G1 E-0.80000"))
        .take(5)
        .collect();
    assert_eq!(after_retract[1], "G1 Z0.600 F7200");
    assert!(after_retract[2].starts_with("G1 X50.000 Y0.000"));
    assert_eq!(after_retract[3], "G1 Z0.200");
    assert_eq!(after_retract[4], "G1 E0.80000 F1800");
    // The first wall ends back at its start, 50 mm from the second.
    assert!((summary.travel_mm - 50.0).abs() < 1e-9);

    // The second island's corner is 1.5 mm from where the first wall
    // ends, under the 2 mm minimum.
    let near = [wall(0.0, 0.0, 10.0, 0), wall(-11.5, 0.0, 10.0, 0)];
    let (text, summary) = print(&one_layer(&near, &[0..1, 1..2], &[]), &settings);
    assert_eq!(summary.retractions, 1);
    // With no retraction the nozzle never rises.
    assert!(!text.contains("Z0.600"));

    let none = PrintSettings {
        retraction_length: 0.0,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&far, &[0..1, 1..2], &[]), &none);
    assert_eq!(summary.retractions, 0);
    assert!(!text.contains("E-"));
}

#[test]
fn walls_follow_the_configured_order_island_by_island() {
    let loops = [
        wall(0.0, 0.0, 10.0, 0),
        wall(0.5, 0.5, 9.0, 1),
        wall(30.0, 0.0, 10.0, 0),
        wall(30.5, 0.5, 9.0, 1),
    ];
    let islands = [0..2, 2..4];
    let (text, _) = print(&one_layer(&loops, &islands, &[]), &basic());
    assert_eq!(
        features(&text),
        ["Inner wall", "Outer wall", "Inner wall", "Outer wall"]
    );

    let outer_first = PrintSettings {
        wall_order: WallOrder::OuterInner,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops, &islands, &[]), &outer_first);
    assert_eq!(
        features(&text),
        ["Outer wall", "Inner wall", "Outer wall", "Inner wall"]
    );

    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SolidInfill)];
    let infill_first = PrintSettings {
        infill_first: true,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops[..2], &[0..2], &fill), &infill_first);
    assert_eq!(
        features(&text),
        ["Internal solid infill", "Inner wall", "Outer wall"]
    );
}

/// Where each stretch of extrusion in `text` begins.
fn extrusion_starts(text: &str) -> Vec<(f64, f64)> {
    let mut found = Vec::new();
    let mut at = (0.0, 0.0);
    let mut extruding = false;
    for line in text.lines().filter(|line| line.starts_with("G1 ")) {
        let to = (
            word(line, 'X').unwrap_or(at.0),
            word(line, 'Y').unwrap_or(at.1),
        );
        let extrudes = word(line, 'E').is_some_and(|e| e > 0.0) && to != at;
        if extrudes && !extruding {
            found.push(at);
        }
        if to != at || word(line, 'Z').is_some() {
            extruding = extrudes;
        }
        at = to;
    }
    found
}

#[test]
fn inner_walls_can_start_further_round_than_the_wall_outside_them() {
    // Three walls nested half a millimetre apart, seams kept in line.
    let loops: Vec<ExtrusionLoop> = (0..3)
        .map(|depth| {
            wall(
                depth as f64 * 0.5,
                depth as f64 * 0.5,
                10.0 - depth as f64,
                depth,
            )
        })
        .collect();
    let aligned = PrintSettings {
        seam: Seam::Aligned,
        ..basic()
    };
    let staggered = PrintSettings {
        staggered_inner_seams: true,
        ..aligned.clone()
    };
    let starts = |settings: &PrintSettings| {
        let (text, _) = print(&one_layer(&loops, &[0..3], &[]), settings);
        extrusion_starts(&text)
    };
    let (plain, moved) = (starts(&aligned), starts(&staggered));
    assert_eq!((plain.len(), moved.len()), (3, 3));
    // Laid innermost first: the outer wall, last, starts where it did.
    assert_eq!(plain[2], moved[2]);
    // Each inner wall starts on along its own side by its width or more,
    // the deeper one further, and still on its own loop.
    let width = flow().width;
    let shift =
        |index: usize| (moved[index].0 - plain[index].0).hypot(moved[index].1 - plain[index].1);
    assert!(
        shift(1) >= width - 1e-3 && shift(1) < 2.0 * width,
        "{}",
        shift(1)
    );
    assert!(
        shift(0) >= shift(1) - 1e-3 && shift(0) < 4.0 * width,
        "{}",
        shift(0)
    );
    for (index, depth) in [(0, 2.0), (1, 1.0)] {
        let (x, y) = moved[index];
        let (low, high) = (depth * 0.5, 10.0 - depth * 0.5);
        let on_side = |v: f64| (v - low).abs() < 1e-3 || (v - high).abs() < 1e-3;
        assert!(on_side(x) || on_side(y), "({x}, {y})");
    }
}

#[test]
fn a_random_seam_starts_each_layer_somewhere_else() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let random = PrintSettings {
        seam: Seam::Random,
        ..basic()
    };
    let layer = one_layer(&loops, &[0..1], &[])[0];
    let layers: Vec<Layer<'_>> = (1..=6)
        .map(|index| Layer {
            z: 0.2 * f64::from(index),
            ..layer
        })
        .collect();
    let (text, _) = print(&layers, &random);
    let mut starts = extrusion_starts(&text);
    assert_eq!(starts.len(), 6);
    starts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    starts.dedup();
    assert!(starts.len() >= 5, "{starts:?}");
    // And the same file again another time.
    assert_eq!(print(&layers, &random).0, text);
}

#[test]
fn a_small_wall_loop_goes_at_its_own_speed() {
    // A wall 40 mm round and one 8 mm round, on the second layer.
    let loops = [wall(0.0, 0.0, 10.0, 0), wall(30.0, 0.0, 2.0, 0)];
    let islands = [0..1, 1..2];
    let speeds = |settings: &PrintSettings| -> Vec<f64> {
        let layer = one_layer(&loops, &islands, &[])[0];
        let layers = [layer, Layer { z: 0.4, ..layer }];
        let (text, _) = print(&layers, settings);
        let second = text.split(";LAYER_CHANGE").last().unwrap().to_owned();
        // The feed rate each wall's first extruding move is made at.
        let mut feed = 0.0;
        let mut found = Vec::new();
        let mut fresh = false;
        for line in second.lines() {
            if line.starts_with(";TYPE:") {
                fresh = true;
            }
            if !line.starts_with("G1 ") {
                continue;
            }
            feed = word(line, 'F').unwrap_or(feed);
            let extrudes = word(line, 'E').is_some_and(|e| e > 0.0) && word(line, 'X').is_some();
            if extrudes && fresh {
                found.push(feed / 60.0);
                fresh = false;
            } else if !extrudes && word(line, 'X').is_some() {
                fresh = true;
            }
        }
        // Whichever island is nearer goes first: slowest last here.
        found.sort_by(|a, b| b.total_cmp(a));
        found
    };
    let base = PrintSettings {
        outer_wall_speed: 100.0,
        ..basic()
    };
    assert_eq!(speeds(&base), [100.0, 100.0]);
    // Loops up to a circle of 2 mm radius, 12.6 mm round: the small one.
    let small = PrintSettings {
        small_perimeter_threshold: 2.0,
        small_perimeter_speed: FloatOrPercent::Percent(30.0),
        ..base.clone()
    };
    assert_eq!(speeds(&small), [100.0, 30.0]);
    let fixed = PrintSettings {
        small_perimeter_speed: FloatOrPercent::Value(12.0),
        ..small.clone()
    };
    assert_eq!(speeds(&fixed), [100.0, 12.0]);
    // At nothing, half the outer wall's speed.
    let unset = PrintSettings {
        small_perimeter_speed: FloatOrPercent::Value(0.0),
        ..small.clone()
    };
    assert_eq!(speeds(&unset), [100.0, 50.0]);
}

/// The x each wall in `text` starts at, less the island's own: 0 for the
/// outermost of [`wall`]'s squares nested half a millimetre apart.
fn wall_depths(text: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut at = (0.0, 0.0);
    let mut extruding = false;
    for line in text.lines().filter(|line| line.starts_with("G1 ")) {
        let to = (
            word(line, 'X').unwrap_or(at.0),
            word(line, 'Y').unwrap_or(at.1),
        );
        let extrudes = word(line, 'E').is_some_and(|e| e > 0.0) && to != at;
        if extrudes && !extruding {
            found.push((at.0.min(at.1) / 0.5).round() as usize);
        }
        // A wall also begins after a move, or a step up to the next layer.
        if to != at || word(line, 'Z').is_some() {
            extruding = extrudes;
        }
        at = to;
    }
    found
}

#[test]
fn the_outer_wall_can_go_between_the_inner_ones() {
    // Four walls nested half a millimetre apart, and three, and two.
    let nest = |count: usize| -> Vec<ExtrusionLoop> {
        (0..count)
            .map(|depth| {
                wall(
                    depth as f64 * 0.5,
                    depth as f64 * 0.5,
                    10.0 - depth as f64,
                    depth,
                )
            })
            .collect()
    };
    let sandwich = PrintSettings {
        wall_order: WallOrder::InnerOuterInner,
        ..basic()
    };
    let second_layer = |loops: &[ExtrusionLoop]| {
        let islands = [0..loops.len()];
        let layer = one_layer(loops, &islands, &[])[0];
        let layers = [layer, Layer { z: 0.4, ..layer }];
        let (text, _) = print(&layers, &sandwich);
        let depths = wall_depths(&text);
        let (first, second) = depths.split_at(loops.len());
        (first.to_vec(), second.to_vec())
    };
    // The first layer goes inner wall first; above it, the walls behind
    // the first inner wall, innermost first, then the outer, then the
    // first inner one.
    assert_eq!(second_layer(&nest(4)), (vec![3, 2, 1, 0], vec![3, 2, 0, 1]));
    assert_eq!(second_layer(&nest(3)), (vec![2, 1, 0], vec![2, 0, 1]));
    // Two walls have no inner one to go first: outer, then inner.
    assert_eq!(second_layer(&nest(2)), (vec![1, 0], vec![0, 1]));
}

/// For each stretch of extrusion in `text`, whether it runs
/// counter-clockwise: the sign of the area its moves enclose.
fn runs_counter_clockwise(text: &str) -> Vec<bool> {
    let (mut at, mut run, mut found) = ((0.0, 0.0), Vec::new(), Vec::new());
    let mut close = |run: &mut Vec<(f64, f64)>| {
        if run.len() >= 3 {
            let area: f64 = (0..run.len())
                .map(|i| {
                    let (a, b) = (run[i], run[(i + 1) % run.len()]);
                    a.0 * b.1 - b.0 * a.1
                })
                .sum();
            found.push(area > 0.0);
        }
        run.clear();
    };
    for line in text.lines().filter(|line| line.starts_with("G1 ")) {
        let to = (
            word(line, 'X').unwrap_or(at.0),
            word(line, 'Y').unwrap_or(at.1),
        );
        if word(line, 'E').is_some_and(|e| e > 0.0) && to != at {
            if run.is_empty() {
                run.push(at);
            }
            run.push(to);
        } else if to != at {
            close(&mut run);
        }
        at = to;
    }
    close(&mut run);
    found
}

#[test]
fn every_wall_is_laid_the_same_way_round() {
    // A part with a hole in it: the wall round the hole is held
    // clockwise, and laid counter-clockwise like the wall round the part.
    let mut hole = wall(3.0, 3.0, 4.0, 0);
    hole.polygon.points.reverse();
    hole.is_hole = true;
    let loops = [wall(0.0, 0.0, 10.0, 0), hole];
    let (text, _) = print(&one_layer(&loops, &[0..2], &[]), &basic());
    assert_eq!(runs_counter_clockwise(&text), [true, true]);
    let same = PrintSettings {
        wall_direction: WallDirection::CounterClockwise,
        ..basic()
    };
    assert_eq!(print(&one_layer(&loops, &[0..2], &[]), &same).0, text);
    let clockwise = PrintSettings {
        wall_direction: WallDirection::Clockwise,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops, &[0..2], &[]), &clockwise);
    assert_eq!(runs_counter_clockwise(&text), [false, false]);

    // A wall marked to turn about over an overhang does, unless the way
    // round is set outright. The wall round the hole is laid first.
    let mut marked = loops.clone();
    marked[1].clockwise = true;
    let (text, _) = print(&one_layer(&marked, &[0..2], &[]), &basic());
    assert_eq!(runs_counter_clockwise(&text), [false, true]);
    let (text, _) = print(&one_layer(&marked, &[0..2], &[]), &same);
    assert_eq!(runs_counter_clockwise(&text), [true, true]);
}

#[test]
fn walls_over_air_are_laid_ahead_of_their_islands_loops() {
    let loops = [
        wall(0.0, 0.0, 10.0, 0),
        wall(0.5, 0.5, 9.0, 1),
        wall(30.0, 0.0, 10.0, 0),
    ];
    let islands = [0..2, 2..3];
    // Given after the fill, and in the second island's case with it.
    let mut over_air = [
        line((2.0, 3.0), (8.0, 3.0), Role::OverhangPerimeter),
        line((8.0, 3.5), (2.0, 3.5), Role::OverhangPerimeter),
    ];
    over_air[1].in_turn = true;
    let fill = [
        line((2.0, 2.0), (8.0, 2.0), Role::SolidInfill),
        over_air[0].clone(),
        over_air[1].clone(),
        line((32.0, 2.0), (38.0, 2.0), Role::SolidInfill),
    ];
    let (text, _) = print(&one_layer(&loops, &islands, &fill), &basic());
    assert_eq!(
        features(&text),
        [
            "Overhang wall",
            "Inner wall",
            "Outer wall",
            "Internal solid infill",
            "Outer wall",
            "Internal solid infill"
        ]
    );
    // In the order given, from the first one's first point.
    let laid: Vec<(f64, f64)> = starts(&text)
        .into_iter()
        .filter(|(feature, _, _)| *feature == "Overhang wall")
        .map(|(_, x, y)| (x, y))
        .collect();
    assert_eq!(laid, [(2.0, 3.0)]);

    let infill_first = PrintSettings {
        infill_first: true,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops[..2], &[0..2], &fill[..3]), &infill_first);
    assert_eq!(
        features(&text),
        [
            "Internal solid infill",
            "Overhang wall",
            "Inner wall",
            "Outer wall"
        ]
    );
}

#[test]
fn speeds_follow_role_layer_and_the_melt_limit() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SparseInfill)];
    let layers = [
        Layer {
            z: 0.2,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            shelter: &[],
            below: &[],
        },
        Layer {
            z: 0.4,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            shelter: &[],
            below: &[],
        },
    ];
    let feed_of = |text: &str, layer: usize, feature: &str| -> f64 {
        let layer_text = text.split(";LAYER_CHANGE").nth(layer + 1).unwrap();
        let line = layer_text
            .lines()
            .skip_while(|line| *line != format!(";TYPE:{feature}"))
            .find(|l| l.contains(" E"));
        word(line.unwrap(), 'F').unwrap()
    };
    let (text, _) = print(&layers, &basic());
    assert_eq!(
        (
            feed_of(&text, 0, "Outer wall"),
            feed_of(&text, 0, "Sparse infill")
        ),
        (1800.0, 3600.0)
    );
    assert_eq!(
        (
            feed_of(&text, 1, "Outer wall"),
            feed_of(&text, 1, "Sparse infill")
        ),
        (3600.0, 6000.0)
    );

    // 4 mm³/s through a 0.45 x 0.2 line is about 47.5 mm/s.
    let limited = PrintSettings {
        filament_max_volumetric_speed: 4.0,
        ..basic()
    };
    let (text, _) = print(&layers, &limited);
    let expected = (4.0 / flow().mm3_per_mm() * 60.0).round();
    assert_eq!(feed_of(&text, 1, "Sparse infill"), expected);
    assert_eq!(feed_of(&text, 0, "Outer wall"), 1800.0);
}

#[test]
fn temperatures_change_after_the_first_layer() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let layer = |z| Layer {
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops: &loops,
        islands: &[0..1],
        paths: &[],
        shelter: &[],
        below: &[],
    };
    let settings = PrintSettings {
        nozzle_temperature: 210.0,
        nozzle_temperature_initial_layer: 220.0,
        bed_temperature: Some(55.0),
        bed_temperature_initial_layer: Some(60.0),
        filament_density: Some(1.24),
        ..basic()
    };
    let (text, summary) = print(&[layer(0.2), layer(0.4), layer(0.6)], &settings);
    let commands: Vec<&str> = text.lines().filter(|line| line.starts_with('M')).collect();
    assert_eq!(
        commands,
        [
            "M140 S60",
            "M104 S220",
            "M190 S60",
            "M109 S220",
            "M83 ; use relative distances for extrusion",
            "M106 S0",
            // The fan for the second layer is set just ahead of it.
            "M106 S255",
            "M104 S210",
            "M140 S55",
            "M106 S0",
            "M104 S0",
            "M140 S0",
            "M84",
        ]
    );
    let grams = summary.filament_mm3 / 1000.0 * 1.24;
    assert!(text.contains(&format!("; filament used [g] = {grams:.2}")));

    // With no bed type known, the bed is left alone.
    let (text, _) = print(&[layer(0.2)], &basic());
    assert!(!text.contains("M190") && text.contains("M104 S200"));
}

#[test]
fn fill_lines_are_taken_nearest_end_first() {
    // Rows given in an order that would zigzag across the part.
    let fill = [
        line((0.0, 0.0), (10.0, 0.0), Role::SolidInfill),
        line((0.0, 2.0), (10.0, 2.0), Role::SolidInfill),
        line((0.0, 1.0), (10.0, 1.0), Role::SolidInfill),
    ];
    let (text, summary) = print(&one_layer(&[], &[], &fill), &basic());
    let rows: Vec<f64> = moves(&text)
        .filter(|l| l.contains(" E"))
        .filter_map(|line| word(line, 'Y'))
        .collect();
    assert_eq!(rows, [0.0, 1.0, 2.0]);
    // Each row starts where the last one ended: two 1 mm hops, no retraction but the last.
    assert!((summary.travel_mm - 2.0).abs() < 1e-9);
    assert_eq!(summary.retractions, 1);
    assert_eq!(print(&one_layer(&[], &[], &fill), &basic()).0, text);
}

#[test]
fn moves_inside_the_shelter_can_skip_retraction() {
    // Two fill lines 6 mm apart, far over the 2 mm minimum.
    let fill = [
        line((1.0, 1.0), (9.0, 1.0), Role::SparseInfill),
        line((9.0, 7.0), (1.0, 7.0), Role::SparseInfill),
    ];
    let block = Polygon {
        points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    };
    let area = [ExPolygon {
        contour: block,
        holes: Vec::new(),
    }];
    let layer = [Layer {
        z: 0.2,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops: &[],
        islands: &[],
        paths: &fill,
        shelter: &area,
        below: &[],
    }];

    let plain = basic();
    assert_eq!(print(&layer, &plain).1.retractions, 2);
    let reduced = PrintSettings {
        reduce_infill_retraction: true,
        ..basic()
    };
    let (text, summary) = print(&layer, &reduced);
    assert_eq!(summary.retractions, 1);
    assert!(!text.contains("Z0.600"));

    // The same move with a hole in the way retracts as usual.
    let mut gap = Polygon {
        points: [(8.0, 3.0), (10.0, 3.0), (10.0, 5.0), (8.0, 5.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    };
    gap.points.reverse();
    let holed = [ExPolygon {
        contour: area[0].contour.clone(),
        holes: vec![gap],
    }];
    let layer = [Layer {
        shelter: &holed,
        below: &[],
        ..layer[0]
    }];
    assert_eq!(print(&layer, &reduced).1.retractions, 2);
}

/// Where each feature's first line starts, in the order printed.
fn starts(text: &str) -> Vec<(&str, f64, f64)> {
    let (mut at, mut found) = ((0.0, 0.0), Vec::new());
    let mut pending = None;
    for line in text.lines() {
        if let Some(kind) = line.strip_prefix(";TYPE:") {
            pending = Some(kind);
        } else if line.starts_with("G1 ") {
            let before = at;
            at = (
                word(line, 'X').unwrap_or(at.0),
                word(line, 'Y').unwrap_or(at.1),
            );
            if word(line, 'E').is_some_and(|e| e > 0.0)
                && at != before
                && let Some(kind) = pending.take()
            {
                found.push((kind, before.0, before.1));
            }
        }
    }
    found
}

#[test]
fn an_island_is_finished_before_the_next_is_begun() {
    // Two squares 30 mm apart, each with a wall, a gap line and fill.
    let loops = [wall(0.0, 0.0, 10.0, 0), wall(30.0, 0.0, 10.0, 0)];
    let fill = [
        line((2.0, 3.0), (8.0, 3.0), Role::GapFill),
        line((32.0, 3.0), (38.0, 3.0), Role::GapFill),
        line((2.0, 5.0), (8.0, 5.0), Role::SolidInfill),
        line((32.0, 5.0), (38.0, 5.0), Role::SolidInfill),
    ];
    let (text, _) = print(&one_layer(&loops, &[0..1, 1..2], &fill), &basic());
    let order: Vec<(&str, bool)> = starts(&text)
        .into_iter()
        .map(|(kind, x, _)| (kind, x > 20.0))
        .collect();
    assert_eq!(
        order,
        [
            ("Outer wall", false),
            ("Gap infill", false),
            ("Internal solid infill", false),
            ("Outer wall", true),
            ("Gap infill", true),
            ("Internal solid infill", true),
        ]
    );

    // With the fill first, it still goes island by island.
    let first = PrintSettings {
        infill_first: true,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops, &[0..1, 1..2], &fill), &first);
    let sides: Vec<bool> = starts(&text)
        .into_iter()
        .map(|(_, x, _)| x > 20.0)
        .collect();
    assert_eq!(sides, [false, false, false, true, true, true]);

    // A thin part's wall that is not a loop goes with the walls, after
    // the loops, whichever of walls and fill comes first.
    let with_thin = [
        line((2.0, 7.0), (8.0, 7.0), Role::ExternalPerimeter),
        fill[0].clone(),
        fill[2].clone(),
    ];
    for (settings, expected) in [
        (
            basic(),
            [
                "Outer wall",
                "Outer wall",
                "Gap infill",
                "Internal solid infill",
            ],
        ),
        (
            first,
            [
                "Gap infill",
                "Internal solid infill",
                "Outer wall",
                "Outer wall",
            ],
        ),
    ] {
        let (text, _) = print(&one_layer(&loops[..1], &[0..1], &with_thin), &settings);
        let kinds: Vec<&str> = starts(&text).into_iter().map(|(kind, ..)| kind).collect();
        // The loop and the line share a heading where one follows the other.
        let mut wanted = expected.to_vec();
        wanted.dedup();
        assert_eq!(kinds, wanted);
    }
}

#[test]
fn lines_laid_in_turn_keep_their_order_and_direction() {
    // Three rows, given bottom, top, middle, each from left to right.
    let rows = |in_turn: bool| {
        [1.0, 9.0, 5.0].map(|y| ExtrusionPath {
            in_turn: in_turn && y != 1.0,
            part: 0,
            ..line((1.0, y), (9.0, y), Role::TopSolidInfill)
        })
    };
    // (where each row starts, and its height)
    let laid = |fill: &[ExtrusionPath]| -> Vec<(f64, f64)> {
        let (text, _) = print(&one_layer(&[], &[], fill), &basic());
        let mut at = (0.0, 0.0);
        let mut found = Vec::new();
        for line in moves(&text) {
            let before = at;
            at = (
                word(line, 'X').unwrap_or(at.0),
                word(line, 'Y').unwrap_or(at.1),
            );
            if word(line, 'E').is_some_and(|e| e > 0.0) && at != before {
                found.push((before.0, before.1));
            }
        }
        found
    };
    // Free, the nearest end is taken each time: up the rows, to and fro.
    assert_eq!(laid(&rows(false)), [(1.0, 1.0), (9.0, 5.0), (1.0, 9.0)]);
    // In turn, they go down as given.
    assert_eq!(laid(&rows(true)), [(1.0, 1.0), (1.0, 9.0), (1.0, 5.0)]);

    // A run goes to the island its first line lies in, all of it.
    let loops = [wall(0.0, 0.0, 10.0, 0), wall(30.0, 0.0, 10.0, 0)];
    let mut fill = rows(true).to_vec();
    fill[2].points = vec![Point2::from_mm(32.0, 5.0), Point2::from_mm(38.0, 5.0)];
    let layer = one_layer(&loops, &[0..1, 1..2], &fill);
    assert_eq!(homes(&layer[0]), [0, 0, 0]);
    fill[2].in_turn = false;
    let layer = one_layer(&loops, &[0..1, 1..2], &fill);
    assert_eq!(homes(&layer[0]), [0, 0, 1]);
}

#[test]
fn fill_is_laid_area_by_area_whatever_its_kind() {
    // Three areas in a row: top fill on the left and on the right, solid
    // fill between them, two lines each. Given kind by kind.
    let area = |x: f64, role: Role, part: u32| {
        [1.0, 2.0].map(|y| ExtrusionPath {
            part,
            ..line((x, y), (x + 5.0, y), role)
        })
    };
    let fill: Vec<ExtrusionPath> = [
        area(0.0, Role::TopSolidInfill, 1),
        area(20.0, Role::TopSolidInfill, 2),
        area(10.0, Role::SolidInfill, 3),
    ]
    .concat();
    let (text, _) = print(&one_layer(&[], &[], &fill), &basic());
    // Left to right, the solid area in its place between the two tops,
    // and both lines of an area before the next area is begun.
    let xs: Vec<f64> = moves(&text)
        .filter(|line| word(line, 'E').is_some_and(|e| e > 0.0))
        .filter_map(|line| word(line, 'X'))
        .map(|x| (x / 10.0).floor())
        .collect();
    assert_eq!(xs, [0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
    assert_eq!(
        features(&text),
        ["Top surface", "Internal solid infill", "Top surface"]
    );

    // Lines that are no part of an area keep to their kind: gap fill
    // first, as it is given, then the fill.
    let mut with_gaps = vec![line((22.0, 5.0), (24.0, 5.0), Role::GapFill)];
    with_gaps.extend(fill);
    let (text, _) = print(&one_layer(&[], &[], &with_gaps), &basic());
    assert_eq!(features(&text)[0], "Gap infill");
}

#[test]
fn fill_goes_with_the_smallest_island_that_holds_it() {
    // A small square standing in the hole of a large one, and a line
    // outside both.
    let mut hole = wall(10.0, 10.0, 20.0, 0);
    hole.polygon.points.reverse();
    hole.is_hole = true;
    let loops = [wall(0.0, 0.0, 40.0, 0), hole, wall(15.0, 15.0, 10.0, 0)];
    let fill = [
        line((17.0, 20.0), (23.0, 20.0), Role::SolidInfill),
        line((2.0, 5.0), (38.0, 5.0), Role::SolidInfill),
        line((50.0, 5.0), (60.0, 5.0), Role::SolidInfill),
    ];
    let layer = one_layer(&loops, &[0..2, 2..3], &fill);
    assert_eq!(homes(&layer[0]), [1, 0, 2]);
}

#[test]
fn walls_are_laid_from_the_inside_out_each_next_to_its_neighbour() {
    // Two walls round a square and two round the hole in it.
    let ring = |x: f64, side: f64, depth: usize, is_hole: bool| {
        let mut wall = wall(x, x, side, depth);
        if is_hole {
            wall.polygon.points.reverse();
            wall.is_hole = true;
        }
        wall
    };
    let loops = [
        ring(0.0, 40.0, 0, false),
        ring(15.0, 10.0, 0, true),
        ring(0.5, 39.0, 1, false),
        ring(14.5, 11.0, 1, true),
    ];
    // The hole's inner wall and then its outer, and the same for the
    // outline: never the two inner walls one after the other.
    assert_eq!(nested(&loops), [3, 1, 2, 0]);
    // A second hole is finished before the outline is begun.
    let more = [
        loops.to_vec(),
        vec![ring(30.0, 4.0, 0, true), ring(29.5, 5.0, 1, true)],
    ]
    .concat();
    assert_eq!(nested(&more), [3, 1, 5, 4, 2, 0]);
    // A wall with nothing round it is still laid.
    assert_eq!(nested(&loops[1..2]), [0]);

    let (text, _) = print(&one_layer(&loops, &[0..4], &[]), &basic());
    assert_eq!(
        features(&text),
        ["Inner wall", "Outer wall", "Inner wall", "Outer wall"]
    );
    let outer_first = PrintSettings {
        wall_order: WallOrder::OuterInner,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops, &[0..4], &[]), &outer_first);
    assert_eq!(
        features(&text),
        ["Outer wall", "Inner wall", "Outer wall", "Inner wall"]
    );
}

#[test]
fn a_move_off_an_overhanging_wall_retracts_even_in_the_shelter() {
    let block = Polygon {
        points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    };
    let area = [ExPolygon {
        contour: block,
        holes: Vec::new(),
    }];
    let reduced = PrintSettings {
        reduce_infill_retraction: true,
        ..basic()
    };
    let count = |first: Role| {
        let fill = [
            line((1.0, 1.0), (9.0, 1.0), first),
            line((9.0, 7.0), (1.0, 7.0), Role::SparseInfill),
        ];
        let layer = [Layer {
            shelter: &area,
            ..one_layer(&[], &[], &fill)[0]
        }];
        print(&layer, &reduced).1.retractions
    };
    assert_eq!(count(Role::SolidInfill), 1);
    assert_eq!(count(Role::OverhangPerimeter), 2);
}

/// A wall and one fill line on each of `count` layers, 0.2 mm apart.
fn printed(count: usize, settings: &PrintSettings) -> String {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SparseInfill)];
    let layers: Vec<Layer<'_>> = (0..count)
        .map(|index| Layer {
            z: 0.2 * (index + 1) as f64,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            tower: &[],
            around: &[],
            support: &[],
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            shelter: &[],
            below: &[],
        })
        .collect();
    print(&layers, settings).0
}

fn profile(templates: &[(&str, &str)]) -> PrintSettings {
    let mut settings = PrintSettings {
        retract_when_changing_layer: true,
        ..basic()
    };
    for (name, text) in templates {
        let slot = match *name {
            "start" => &mut settings.machine_start_gcode,
            "end" => &mut settings.machine_end_gcode,
            "before" => &mut settings.before_layer_change_gcode,
            "after" => &mut settings.layer_change_gcode,
            "filament_start" => &mut settings.filament_start_gcode,
            _ => &mut settings.filament_end_gcode,
        };
        *slot = Some((*text).to_owned());
    }
    settings
}

#[test]
fn the_profiles_templates_replace_the_generic_preamble() {
    let settings = profile(&[
        (
            "start",
            "PRINT_START TOTAL={total_layer_count}\nG1 X185 E15 F360",
        ),
        ("filament_start", "; filament start"),
        ("before", ";BEFORE [layer_z]\nG92 E0"),
        ("after", ";AFTER {layer_num+1} of {total_layer_count}"),
        ("filament_end", "; filament end\n"),
        (
            "end",
            "{if max_layer_z > 0.5}G1 Z{max_layer_z+2}{endif}\nPRINT_END",
        ),
    ]);
    let text = printed(3, &settings);
    // What the printer is given to run before the first layer.
    let start: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != "; EXECUTABLE_BLOCK_START")
        .take_while(|line| *line != ";LAYER_CHANGE")
        .collect();
    assert_eq!(
        start[1..],
        [
            "M73 P0 R0",
            ";TYPE:Custom",
            "PRINT_START TOTAL=3",
            "G1 X185 E15 F360",
            "G90",
            "G21",
            "M83 ; use relative distances for extrusion",
            "; filament start",
            "M106 S0",
        ]
    );
    // A project set up with more than one filament names the tool it
    // starts with, even when it prints in the first alone.
    let two = PrintSettings {
        filaments: 2,
        ..settings.clone()
    };
    let text_for_two = printed(3, &two);
    let before_layers = text_for_two.split(";LAYER_CHANGE").next().unwrap();
    assert!(before_layers.lines().any(|line| line == "T0"));
    assert!(!text.contains("NOT PRINTER-READY") && !text.contains("G28"));

    // Each layer: the profile's own lines, a retraction between them, then the rise.
    let second: Vec<&str> = text
        .split(";LAYER_CHANGE\n")
        .nth(2)
        .unwrap()
        .lines()
        .take(7)
        .collect();
    assert_eq!(
        second[..4],
        [";Z:0.400", ";HEIGHT:0.200", ";BEFORE 0.4", "G92 E0"]
    );
    assert_eq!(second[4], "G1 E-0.80000 F1800");
    assert_eq!(second[5], ";AFTER 2 of 3");
    let end: Vec<&str> = text
        .lines()
        .take_while(|line| *line != "; EXECUTABLE_BLOCK_END")
        .filter(|line| !line.starts_with("M73"))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .take(3)
        .collect();
    // The top layer is at 0.6 mm, so the end G-code lifts to 2.6.
    assert_eq!((end[0], end[2]), ("PRINT_END", "; filament end"));
    let lift: f64 = end[1].strip_prefix("G1 Z").unwrap().parse().unwrap();
    assert!((lift - 2.6).abs() < 1e-9, "{}", end[1]);
}

#[test]
fn start_gcode_leaves_the_nozzle_primed_so_the_first_move_retracts() {
    let with = printed(1, &profile(&[("start", "PRINT_START")]));
    let after_change: Vec<&str> = with
        .split(";HEIGHT:0.200\n")
        .nth(1)
        .unwrap()
        .lines()
        .take(4)
        .collect();
    // Retract, cross at the height the start G-code left the nozzle at,
    // then to just above the layer and down onto it.
    assert_eq!(after_change[0], "G1 E-0.80000 F1800");
    assert!(after_change[1].starts_with("G1 X"));
    assert_eq!(after_change[2..], ["G1 Z0.600", "G1 Z0.200"]);
    // With no start G-code nothing has been extruded yet, so nothing is retracted.
    let without = printed(1, &basic());
    assert!(!without.split(";TYPE:").next().unwrap().contains("E-"));
}

#[test]
fn a_bad_template_names_the_setting_and_line() {
    let settings = profile(&[("start", "G28\nM140 S{bed_temp}")]);
    let error = write(&mut Vec::new(), &[], &settings).unwrap_err();
    assert_eq!(
        error.to_string(),
        "machine_start_gcode, line 2: unknown variable \"bed_temp\""
    );
}

#[test]
fn fan_runs_faster_on_layers_that_take_less_time() {
    let fan = |settings: &PrintSettings| -> Vec<String> {
        printed(3, settings)
            .lines()
            .filter(|line| line.starts_with("M106"))
            .map(str::to_owned)
            .collect()
    };
    // The layer takes under a second, far below the 5 s that calls for full speed.
    assert_eq!(fan(&basic()), ["M106 S0", "M106 S255", "M106 S0"]);
    // Made to count as a long layer, the fan stops, or keeps to its
    // minimum when the profile would rather it did not stop and start.
    let long = PrintSettings {
        fan_cooling_layer_time: 0.1,
        slow_down_layer_time: 0.05,
        ..basic()
    };
    assert_eq!(fan(&long), ["M106 S0", "M106 S0"]);
    let patient = PrintSettings {
        reduce_fan_stop_start_freq: true,
        fan_cooling_layer_time: 0.1,
        slow_down_layer_time: 0.05,
        fan_min_speed: 40.0,
        ..basic()
    };
    assert_eq!(fan(&patient), ["M106 S0", "M106 S102", "M106 S0"]);
    // In between, the speed is scaled.
    let scaled = PrintSettings {
        fan_cooling_layer_time: 1.5,
        slow_down_layer_time: 0.5,
        fan_min_speed: 0.0,
        ..basic()
    };
    let speed: u32 = fan(&scaled)[1]
        .strip_prefix("M106 S")
        .unwrap()
        .parse()
        .unwrap();
    assert!(speed > 0 && speed < 255, "fan at {speed}");
    let never = PrintSettings {
        close_fan_the_first_x_layers: 9,
        ..basic()
    };
    assert_eq!(fan(&never), ["M106 S0", "M106 S0"]);
}

#[test]
fn acceleration_follows_the_feature_and_the_first_layer() {
    let limits = |settings: &PrintSettings| -> Vec<String> {
        let text = printed(2, settings);
        text.lines()
            .filter(|line| line.starts_with("SET_VELOCITY") || line.starts_with("M204"))
            .map(str::to_owned)
            .collect()
    };
    let klipper = PrintSettings {
        klipper: true,
        accel_to_decel_factor: Some(0.5),
        default_acceleration: 10000.0,
        outer_wall_acceleration: 5000.0,
        initial_layer_acceleration: 500.0,
        ..basic()
    };
    assert_eq!(
        limits(&klipper),
        [
            // The whole first layer, travel included, is gentle.
            "SET_VELOCITY_LIMIT ACCEL=500 ACCEL_TO_DECEL=250 MINIMUM_CRUISE_RATIO=0.5",
            // Second layer: travel, the outer wall, then fill at the default.
            "SET_VELOCITY_LIMIT ACCEL=10000 ACCEL_TO_DECEL=5000 MINIMUM_CRUISE_RATIO=0.5",
            "SET_VELOCITY_LIMIT ACCEL=5000 ACCEL_TO_DECEL=2500 MINIMUM_CRUISE_RATIO=0.5",
            "SET_VELOCITY_LIMIT ACCEL=10000 ACCEL_TO_DECEL=5000 MINIMUM_CRUISE_RATIO=0.5",
        ]
    );
    let marlin = PrintSettings {
        klipper: false,
        accel_to_decel_factor: None,
        ..klipper.clone()
    };
    assert_eq!(limits(&marlin)[..2], ["M204 S500", "M204 S10000"]);
    // With no accelerations set, the printer's own are left alone.
    assert!(limits(&basic()).is_empty());
}

#[test]
fn pressure_advance_is_set_once_before_the_first_layer() {
    let klipper = PrintSettings {
        klipper: true,
        pressure_advance: Some(0.02),
        ..basic()
    };
    let text = printed(1, &klipper);
    assert_eq!(
        text.matches("SET_PRESSURE_ADVANCE ADVANCE=0.02; Override pressure advance value")
            .count(),
        1
    );
    assert!(text.find("SET_PRESSURE_ADVANCE").unwrap() < text.find(";LAYER_CHANGE").unwrap());
    let marlin = PrintSettings {
        pressure_advance: Some(0.05),
        ..basic()
    };
    assert!(printed(1, &marlin).contains("M900 K0.05"));
    assert!(!printed(1, &basic()).contains("M900"));
}

/// The lines from the first that starts with `marker`, as far as `count`.
fn from<'a>(text: &'a str, marker: &str, count: usize) -> Vec<&'a str> {
    text.lines()
        .skip_while(|line| !line.starts_with(marker))
        .take(count)
        .collect()
}

/// The feedrate of the first line extruded for a feature.
fn feed_of(text: &str, feature: &str) -> Option<f64> {
    let line = text
        .lines()
        .skip_while(|line| *line != format!(";TYPE:{feature}"))
        .find(|line| line.contains(" E"))?;
    word(line, 'F')
}

#[test]
fn a_wall_stops_short_of_its_start_by_the_seam_gap() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let settings = PrintSettings {
        seam_gap: 0.25,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&loops, &[0..1], &[]), &settings);
    let last = moves(&text)
        .filter(|line| line.contains(" E") && line.contains(" X"))
        .last();
    assert!(last.unwrap().starts_with("G1 X0.000 Y0.250 E"), "{last:?}");
    let volume = (40.0 - 0.25) * flow().mm3_per_mm();
    assert!((summary.filament_mm3 - volume).abs() < 1e-9);
}

#[test]
fn inner_walls_start_alongside_the_outer_walls_seam() {
    let loops = [wall(0.0, 0.0, 10.0, 0), wall(0.5, 0.5, 9.0, 1)];
    let settings = PrintSettings {
        seam: Seam::Back,
        ..basic()
    };
    let (text, _) = print(&one_layer(&loops, &[0..2], &[]), &settings);
    let starts: Vec<&str> = text
        .lines()
        .zip(text.lines().skip(1))
        .filter(|(_, next)| next.starts_with("G1 Z0.200") || next.starts_with(";TYPE:"))
        .map(|(line, _)| line)
        .filter(|line| line.starts_with("G1 X"))
        .collect();
    // The inner wall is printed first, from the point nearest the outer
    // wall's seam at the back corner.
    assert!(starts[0].starts_with("G1 X9.500 Y9.500"), "{starts:?}");
    assert!(starts[1].starts_with("G1 X10.000 Y10.000"), "{starts:?}");
}

#[test]
fn a_retraction_wipes_the_nozzle_over_what_it_just_printed() {
    let far = [wall(0.0, 0.0, 10.0, 0), wall(50.0, 0.0, 10.0, 0)];
    let wiping = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&far, &[0..1, 1..2], &[]), &wiping);
    // Fully retracted first, then on round the start of the loop at the
    // speed the wall was printed at.
    assert_eq!(
        from(&text, "G1 E-", 5),
        [
            // No feedrate: the wall was printed at the speed of the retraction.
            "G1 E-0.80000",
            ";WIPE_START",
            "G1 F1800",
            "G1 X2.000 Y0.000",
            ";WIPE_END"
        ]
    );
    // The wipe moves the nozzle 2 mm nearer the second wall, and the last
    // retraction of the print wipes too.
    assert!(
        (summary.travel_mm - (2.0 + 48.0 + 2.0)).abs() < 1e-9,
        "{}",
        summary.travel_mm
    );

    // With part of the retraction left for the wipe, it happens on the move.
    let during = PrintSettings {
        retract_before_wipe: 0.7,
        ..wiping.clone()
    };
    let (text, _) = print(&one_layer(&far, &[0..1, 1..2], &[]), &during);
    let lines = from(&text, "G1 E-", 4);
    assert_eq!(
        (lines[0], lines[3]),
        ("G1 E-0.56000", "G1 X2.000 Y0.000 E-0.24000")
    );

    // A line of fill is wiped back the way it came.
    let fill = [
        line((0.0, 0.0), (10.0, 0.0), Role::SolidInfill),
        line((0.0, 20.0), (10.0, 20.0), Role::SolidInfill),
    ];
    let (text, _) = print(&one_layer(&[], &[], &fill), &wiping);
    assert_eq!(from(&text, ";WIPE_START", 3)[2], "G1 X8.000 Y0.000");
}

#[test]
fn a_ramped_lift_climbs_along_the_start_of_the_travel() {
    let settings = PrintSettings {
        lift: Lift::Slope,
        ..basic()
    };
    let far = [wall(0.0, 0.0, 10.0, 0), wall(50.0, 0.0, 10.0, 0)];
    let (text, summary) = print(&one_layer(&far, &[0..1, 1..2], &[]), &settings);
    // 0.4 mm up a 3 degree ramp takes 7.632 mm.
    assert_eq!(
        from(&text, "G1 E-", 5)[1..],
        [
            "G1 X7.632 Y0.000 Z0.600 F7200",
            "G1 X50.000 Y0.000 Z0.600",
            "G1 Z0.200",
            "G1 E0.80000 F1800"
        ]
    );
    assert!((summary.travel_mm - 50.0).abs() < 1e-6);

    // A travel shorter than the ramp climbs all the way along it.
    let near = [wall(0.0, 0.0, 10.0, 0), wall(-13.0, 0.0, 10.0, 0)];
    let (text, _) = print(&one_layer(&near, &[0..1, 1..2], &[]), &settings);
    assert_eq!(from(&text, "G1 E-", 2)[1], "G1 X-3.000 Y0.000 Z0.600 F7200");

    // The Z axis sets the pace when it is the slower one.
    let slow_z = PrintSettings {
        z_speed: 2.0,
        ..settings
    };
    let (text, _) = print(&one_layer(&far, &[0..1, 1..2], &[]), &slow_z);
    let climb = from(&text, "G1 E-", 2)[1];
    let expected = (2.0 * 7.632_f64.hypot(0.4) / 0.4 * 60.0_f64).round();
    assert!(
        (word(climb, 'F').unwrap() - expected).abs() <= 2.0,
        "{climb}"
    );
}

#[test]
fn a_spiral_lift_turns_once_and_needs_to_know_the_bed() {
    let far = [wall(0.0, 0.0, 10.0, 0), wall(50.0, 0.0, 10.0, 0)];
    let settings = PrintSettings {
        lift: Lift::Spiral,
        bed: Some([-20.0, -20.0, 100.0, 100.0]),
        ..basic()
    };
    let (text, _) = print(&one_layer(&far, &[0..1, 1..2], &[]), &settings);
    let turn: Vec<&str> = from(&text, "G1 E-", 200)[1..]
        .iter()
        .copied()
        .take_while(|line| !line.starts_with("G1 X50.000"))
        .collect();
    assert!(turn.len() >= 8 && turn.len() <= 128, "{} steps", turn.len());
    // Rising all the way round, back to where it started.
    let heights: Vec<f64> = turn.iter().map(|line| word(line, 'Z').unwrap()).collect();
    assert!(heights.windows(2).all(|pair| pair[1] > pair[0]));
    assert!(turn.last().unwrap().starts_with("G1 X0.000 Y0.000 Z0.600"));
    // The circle lies to the left of the travel and is 1.217 mm in radius.
    let furthest = turn
        .iter()
        .map(|line| word(line, 'Y').unwrap())
        .fold(0.0, f64::max);
    assert!((furthest - 2.0 * 1.217).abs() < 0.05, "reached Y{furthest}");

    // Off the edge of the bed, or with no bed known, it goes straight up.
    for bed in [Some([-1.0, -1.0, 100.0, 100.0]), None] {
        let (text, _) = print(
            &one_layer(&far, &[0..1, 1..2], &[]),
            &PrintSettings {
                bed,
                ..settings.clone()
            },
        );
        assert_eq!(from(&text, "G1 E-", 2)[1], "G1 Z0.600 F7200");
    }

    // Left to choose, the lift is a spiral between layers and a ramp within one.
    let auto = PrintSettings {
        lift: Lift::Auto,
        retract_when_changing_layer: true,
        ..settings
    };
    let text = printed(2, &auto);
    let second = text.split(";LAYER_CHANGE").nth(2).unwrap();
    let spiral = from(second, "G1 X", 1)[0];
    assert!(word(spiral, 'Z').unwrap() < 0.45, "{spiral}");
    let within = from(second, ";WIPE_END", 1);
    assert!(within.is_empty());
    let ramp = second
        .lines()
        .skip_while(|line| !line.starts_with(";TYPE:Outer wall"))
        .skip_while(|line| !line.starts_with("G1 E-"))
        .nth(1)
        .unwrap();
    assert!(ramp.ends_with("Z0.800 F7200"), "{ramp}");
}

#[test]
fn short_layers_are_slowed_to_give_them_time_to_cool() {
    // 120 mm of wall at the first layer's 30 mm/s takes 4 s; the layer is
    // to take 5.
    let loops = [wall(0.0, 0.0, 30.0, 0)];
    let settings = PrintSettings {
        slow_down_for_layer_cooling: true,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&loops, &[0..1], &[]), &settings);
    let feed = feed_of(&text, "Outer wall").unwrap();
    assert_eq!(feed, (120.0 / 5.005 * 60.0_f64).round());
    assert_eq!(summary.slowed_layers, 1);
    // The time asked for, and the retraction that ends the print.
    let seconds = 5.005 + 0.8 / 30.0;
    assert!(
        (summary.seconds - seconds).abs() < 1e-3,
        "{}",
        summary.seconds
    );

    // A layer long enough is left alone.
    let big = [wall(0.0, 0.0, 50.0, 0)];
    let (text, summary) = print(&one_layer(&big, &[0..1], &[]), &settings);
    assert_eq!(feed_of(&text, "Outer wall"), Some(1800.0));
    assert_eq!(summary.slowed_layers, 0);

    // A tiny one goes no slower than the slowest speed allowed.
    let tiny = [wall(0.0, 0.0, 5.0, 0)];
    let (text, _) = print(&one_layer(&tiny, &[0..1], &[]), &settings);
    assert_eq!(feed_of(&text, "Outer wall"), Some(600.0));

    // Outer walls can be kept at full speed; here that leaves nothing to slow.
    let held = PrintSettings {
        dont_slow_down_outer_wall: true,
        ..settings.clone()
    };
    let (text, summary) = print(&one_layer(&loops, &[0..1], &[]), &held);
    assert_eq!(feed_of(&text, "Outer wall"), Some(1800.0));
    assert_eq!(summary.slowed_layers, 0);

    // Slowed or not, the same plastic goes down.
    let (_, plain) = print(&one_layer(&loops, &[0..1], &[]), &basic());
    let (_, slowed) = print(&one_layer(&loops, &[0..1], &[]), &settings);
    assert_eq!(plain.filament_mm, slowed.filament_mm);
}

#[test]
fn the_auxiliary_fan_comes_on_after_the_first_layers() {
    let settings = PrintSettings {
        auxiliary_fan: true,
        additional_cooling_fan_speed: 50.0,
        ..basic()
    };
    let text = printed(3, &settings);
    let fans: Vec<&str> = text.lines().filter(|l| l.starts_with("M106")).collect();
    assert_eq!(
        fans,
        [
            // Before the start G-code, then again ahead of the first layer.
            "M106 S0",
            "M106 P2 S0",
            "M106 S0",
            "M106 P2 S0",
            "M106 S255",
            "M106 P2 S127",
            "M106 S0",
            "M106 P2 S0",
        ]
    );
    assert!(!printed(3, &basic()).contains("P2"));
}

#[test]
fn skirt_and_brim_go_down_before_the_part() {
    let part = [wall(0.0, 0.0, 10.0, 0)];
    let mut around = [wall(-3.0, -3.0, 16.0, 0), wall(-0.5, -0.5, 11.0, 0)];
    around[0].role = Role::Skirt;
    around[1].role = Role::Brim;
    let layer = [Layer {
        around: &around,
        support: &[],
        ..one_layer(&part, &[0..1], &[])[0]
    }];
    let settings = PrintSettings {
        initial_layer_speed: 25.0,
        ..basic()
    };
    let (text, _) = print(&layer, &settings);
    assert_eq!(features(&text), ["Skirt", "Brim", "Outer wall"]);
    let feed = |feature: &str| feed_of(&text, feature);
    // The skirt has its own speed; the brim goes at the first layer's.
    assert_eq!((feed("Skirt"), feed("Brim")), (Some(3000.0), Some(1500.0)));
}

#[test]
fn supports_go_down_before_the_part_at_their_own_speeds() {
    let part = [wall(0.0, 0.0, 10.0, 0)];
    let support = [
        line((20.0, 0.0), (30.0, 0.0), Role::Support),
        line((20.0, 3.0), (30.0, 3.0), Role::Support),
        line((20.0, 6.0), (30.0, 6.0), Role::SupportInterface),
    ];
    let layers: Vec<Layer<'_>> = [0.2, 0.4]
        .into_iter()
        .map(|z| Layer {
            z,
            support: &support,
            ..one_layer(&part, &[0..1], &[])[0]
        })
        .collect();
    let settings = PrintSettings {
        support_speed: 150.0,
        support_interface_speed: 40.0,
        ..basic()
    };
    let (text, summary) = print(&layers, &settings);
    assert_eq!(
        features(&text),
        [
            "Support",
            "Support interface",
            "Outer wall",
            "Support",
            "Support interface",
            "Outer wall"
        ]
    );
    // On the first layer supports go at the walls' careful speed; above it
    // at their own.
    let second = text.split(";LAYER_CHANGE").nth(2).unwrap();
    assert_eq!(feed_of(&text, "Support"), Some(1800.0));
    assert_eq!(feed_of(second, "Support"), Some(9000.0));
    assert_eq!(feed_of(second, "Support interface"), Some(2400.0));
    // Their plastic is counted with the rest.
    let volume = 2.0 * (40.0 + 30.0) * flow().mm3_per_mm();
    assert!((summary.filament_mm3 - volume).abs() < 1e-9);
}

/// Two filaments' settings for a printer whose profile has a tool-change
/// template, and keeps an idle tool 150 °C cooler.
fn two_tools(change: Option<&str>) -> [PrintSettings; 2] {
    let tool = |temperature: f64, flow_ratio: f64| PrintSettings {
        machine_start_gcode: Some("PRINT_START\nT0".into()),
        change_filament_gcode: change.map(str::to_owned),
        filament_start_gcode: Some("; filament start".into()),
        filament_end_gcode: Some("; filament end".into()),
        nozzle_temperature: temperature,
        nozzle_temperature_initial_layer: temperature + 10.0,
        filament_flow_ratio: flow_ratio,
        retract_length_toolchange: 10.0,
        standby_temperature_delta: Some(-150.0),
        tool_change_temperature_wait: false,
        preheat_time: 30.0,
        tools: 2,
        ..basic()
    };
    [tool(210.0, 1.0), tool(240.0, 0.9)]
}

/// Layers 0.2 mm apart, each printed by the filaments named, in order: a
/// wall for the first of them and one fill line for each of the others.
fn in_filaments(layers: &[&[usize]], settings: &[PrintSettings]) -> (String, Summary) {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((20.0, 0.0), (30.0, 0.0), Role::SolidInfill)];
    let plan: Vec<Layer<'_>> = layers
        .iter()
        .enumerate()
        .flat_map(|(index, filaments)| {
            let (loops, fill) = (&loops, &fill);
            filaments
                .iter()
                .enumerate()
                .map(move |(turn, filament)| Layer {
                    z: 0.2 * (index + 1) as f64,
                    filament: *filament,
                    loops: if turn == 0 { loops } else { &[] },
                    islands: if turn == 0 { &[0..1] } else { &[] },
                    paths: if turn == 0 { &[] } else { fill },
                    ..one_layer(&[], &[], &[])[0]
                })
        })
        .collect();
    let mut out = Vec::new();
    let summary = write_tools(&mut out, &plan, settings).unwrap();
    (String::from_utf8(out).unwrap(), summary)
}

const CHANGE: &str = "; Change Tool[previous_extruder] -> Tool[next_extruder] (layer [layer_num])\n{\n\"M109 S\" + new_filament_temp + \" T\" + next_extruder + \"\n\";\n\"T\" + next_extruder + \"\n\";\n}";

#[test]
fn a_tool_change_goes_through_the_printers_own_gcode() {
    let (text, summary) = in_filaments(&[&[0, 1], &[1, 0]], &two_tools(Some(CHANGE)));
    assert_eq!(summary.tool_changes, 2);
    // The print opens with the template for its first tool, from the tool
    // the start G-code chose, before the first layer.
    let opening = from(&text, "; custom gcode start", 8);
    assert_eq!(
        opening[..7],
        [
            "; custom gcode start",
            "; Change Tool0 -> Tool0 (layer -1)",
            "M109 S220 T0",
            "T0",
            "; custom gcode end",
            ";_FORCE_RESUME_FAN_SPEED",
            "; filament start",
        ]
    );
    assert_eq!(opening[7], "M104 S220 T0 ; set nozzle temperature");

    // The change on the first layer: pull the filament right back, end the
    // old filament, cool its tool, run the template, start the new one.
    let first = text.split(";LAYER_CHANGE").nth(1).unwrap();
    let change: Vec<&str> = first
        .lines()
        .skip_while(|line| *line != "; filament end")
        .take(9)
        .collect();
    assert_eq!(
        change,
        [
            "; filament end",
            "M104 S70 T0 ; set nozzle temperature ;cooldown",
            "; custom gcode start",
            "; Change Tool0 -> Tool1 (layer 0)",
            "M109 S250 T1",
            "T1",
            "; custom gcode end",
            "; filament start",
            "M104 S250 T1 ; set nozzle temperature",
        ]
    );
    let pulled: f64 = first
        .lines()
        .skip_while(|line| *line != ";TYPE:Outer wall")
        .take_while(|line| *line != "; filament end")
        .filter(|line| line.starts_with("G1 E-"))
        .filter_map(|line| word(line, 'E'))
        .sum();
    assert!((pulled + 10.0).abs() < 1e-9, "pulled back {pulled}");
    // The template names the tool, so it is not named again.
    assert_eq!(text.lines().filter(|line| *line == "T1").count(), 1);

    // On the second layer tool 0 comes back: what was pulled out of it is
    // pushed back in, and it prints at its own temperature and flow.
    let second = text.split(";LAYER_CHANGE").nth(2).unwrap();
    assert!(second.contains("; Change Tool1 -> Tool0 (layer 1)"));
    assert!(second.contains("M104 S210 T0 ; set nozzle temperature"));
    let back = second
        .lines()
        .skip_while(|line| *line != "M104 S210 T0 ; set nozzle temperature")
        .find(|line| line.starts_with("G1 E") && !line.starts_with("G1 E-"));
    assert_eq!(word(back.unwrap(), 'E'), Some(10.0), "{back:?}");
    // Tool 1 lays 10% less plastic for the same line.
    let fed = |part: &str, y: f64| -> f64 {
        part.lines()
            .filter(|line| line.starts_with("G1 X30.000") && word(line, 'Y') == Some(y))
            .filter_map(|line| word(line, 'E'))
            .sum()
    };
    let (thin, full) = (fed(first, 0.0), fed(second, 0.0));
    assert!((thin / full - 0.9).abs() < 1e-3, "{thin} against {full}");
    // Tool 1 is not needed again after the second layer, so it is switched off.
    assert!(second.contains("M104 S0 T1 ; set nozzle temperature ;cooldown"));
}

#[test]
fn the_tool_is_named_when_the_profile_does_not() {
    let silent = "; swapping\n";
    let (text, _) = in_filaments(&[&[0, 1]], &two_tools(Some(silent)));
    let layer = text.split(";LAYER_CHANGE").nth(1).unwrap();
    let after: Vec<&str> = from(layer, "; swapping", 3);
    assert_eq!(after[..3], ["; swapping", "; custom gcode end", "T1"]);
    // With no template at all, the tool is simply selected.
    let (text, summary) = in_filaments(&[&[0, 1]], &two_tools(None));
    assert_eq!(summary.tool_changes, 1);
    assert!(text.lines().any(|line| line == "T1") && !text.contains("custom gcode"));
    // A template that needs something unknown stops the export.
    let broken = two_tools(Some("{no_such_thing}"));
    let error = write_tools(&mut Vec::new(), &[], &broken);
    assert!(error.is_ok(), "nothing to print, nothing to expand");
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let plan = [
        one_layer(&loops, &[0..1], &[])[0],
        Layer {
            filament: 1,
            ..one_layer(&loops, &[0..1], &[])[0]
        },
    ];
    let error = write_tools(&mut Vec::new(), &plan, &broken).unwrap_err();
    assert!(
        error.to_string().contains("change_filament_gcode"),
        "{error}"
    );
}

#[test]
fn the_next_tool_is_warmed_ahead_of_its_turn() {
    // One tool prints for a long time before the other is wanted: a slow
    // wall, 160 mm at 2 mm/s.
    // The file is timed as a printer runs it, which takes knowing the printer.
    let slow = |settings: PrintSettings| PrintSettings {
        outer_wall_speed: 2.0,
        initial_layer_speed: 2.0,
        limits: Some(edge_time::Limits {
            flavor: edge_time::Flavor::Klipper,
            max_extruding: 5000.0,
            ..edge_time::Limits::default()
        }),
        ..settings
    };
    let [a, b] = two_tools(Some(CHANGE));
    let settings = [slow(a), slow(b)];
    let loops = [wall(0.0, 0.0, 40.0, 0), wall(50.0, 0.0, 10.0, 0)];
    let fill = [line((20.0, 0.0), (30.0, 0.0), Role::SolidInfill)];
    let plan = [
        one_layer(&loops, &[0..1, 1..2], &[])[0],
        Layer {
            filament: 1,
            ..one_layer(&[], &[], &fill)[0]
        },
    ];
    let mut out = Vec::new();
    write_tools(&mut out, &plan, &settings).unwrap();
    let text = String::from_utf8(out).unwrap();
    let preheat = text
        .lines()
        .position(|line| line.starts_with("M104 S250 T1 ; preheat T1 time:"))
        .expect("no preheat line");
    let change = text
        .lines()
        .position(|line| line.starts_with("; Change Tool0 -> Tool1"))
        .unwrap();
    // Before the change, and after the long wall has started.
    let wall_start = text
        .lines()
        .position(|line| line == ";TYPE:Outer wall")
        .unwrap();
    assert!(wall_start < preheat && preheat < change);
    // No preheating when idle tools are not cooled in the first place.
    let warm = settings.map(|settings| PrintSettings {
        standby_temperature_delta: None,
        ..settings
    });
    let mut out = Vec::new();
    write_tools(&mut out, &plan, &warm).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("preheat") && !text.contains("cooldown"));
}

#[test]
fn the_prime_tower_is_printed_first_and_its_first_line_slowly() {
    let part = [wall(0.0, 0.0, 10.0, 0)];
    let mut slow = line((40.0, 40.0), (60.0, 40.0), Role::PrimeLine);
    slow.flow = Flow::new(0.5, 0.2, 0.4).unwrap();
    let tower = [slow, line((60.0, 40.6), (40.0, 40.6), Role::PrimeTower)];
    let layers: Vec<Layer<'_>> = [0.2, 0.4]
        .into_iter()
        .map(|z| Layer {
            z,
            ram: None,
            enter: &[],
            tower: &tower,
            ..one_layer(&part, &[0..1], &[])[0]
        })
        .collect();
    let (text, _) = print(&layers, &basic());
    // Named once though it has two roles, and ahead of the part.
    assert_eq!(
        features(&text),
        ["Prime tower", "Outer wall", "Prime tower", "Outer wall"]
    );
    // A third of the speed for the first line: of the first layer's 30 mm/s,
    // then of the 90 mm/s the tower is held to.
    let second = text.split(";LAYER_CHANGE").nth(2).unwrap();
    let feeds: Vec<f64> = second
        .lines()
        .filter(|line| line.contains(" Y40.") && line.contains(" E"))
        .filter_map(|line| word(line, 'F'))
        .collect();
    assert_eq!(feeds, [(0.33 * 90.0 * 60.0_f64).round(), 5400.0]);
    assert_eq!(
        feed_of(&text, "Prime tower"),
        Some((0.33 * 30.0 * 60.0_f64).round())
    );
}

#[test]
fn a_tree_branch_s_wall_is_laid_hook_first() {
    // A support line that ends at (10, 0), then a branch's wall whose
    // hook starts at (14, 0) and whose far end, at (12, 0), is the nearer.
    let before = line((0.0, 0.0), (10.0, 0.0), Role::Support);
    let mut hooked = line((14.0, 0.0), (14.0, 1.0), Role::Support);
    hooked.points.extend(
        [(20.0, 1.0), (20.0, -1.0), (12.0, -1.0), (12.0, 0.0)].map(|(x, y)| Point2::from_mm(x, y)),
    );
    let supports = [before, hooked];
    let part = [wall(50.0, 50.0, 10.0, 0)];
    let layer = Layer {
        support: &supports,
        ..one_layer(&part, &[0..1], &[])[0]
    };
    let (text, _) = print(&[layer], &basic());
    let at = |wanted: &str| {
        text.lines()
            .position(|line| line.starts_with(wanted))
            .unwrap_or_else(|| panic!("no line starting {wanted}"))
    };
    // It is gone to at its hook and ends at its far end.
    let hook = at("G1 X14.000 Y0.000");
    let end = at("G1 X12.000 Y0.000 E");
    assert!(hook < end);
    assert!(!text.lines().nth(hook).unwrap().contains(" E"));

    // A plain support line with its ends far apart is still taken from
    // whichever end is nearer.
    let supports = [
        line((0.0, 0.0), (10.0, 0.0), Role::Support),
        line((40.0, 0.0), (12.0, 0.0), Role::Support),
    ];
    let layer = Layer {
        support: &supports,
        ..one_layer(&part, &[0..1], &[])[0]
    };
    let (text, _) = print(&[layer], &basic());
    assert!(text.contains("G1 X40.000 Y0.000 E"));
}

#[test]
fn a_rammed_filament_is_laid_by_the_tool_being_put_away() {
    let [a, b] = two_tools(Some(CHANGE));
    let settings = [a, b];
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    // A ramming line 1 mm wide and 2.6 mm long on the tower, at 104 mm/s.
    let mut rammed = line((40.75, 40.85), (43.35, 40.85), Role::PrimeTower);
    rammed.flow = Flow::new(1.0, 0.2, 0.4).unwrap();
    let ram = [rammed];
    let mut slow = line((43.35, 41.1), (60.0, 41.1), Role::PrimeLine);
    slow.flow = Flow::new(0.5, 0.2, 0.4).unwrap();
    let tower = [slow];
    let enter = [Point2::from_mm(38.5, 41.45), Point2::from_mm(40.25, 41.45)];
    let plan = [
        one_layer(&loops, &[0..1], &[])[0],
        Layer {
            filament: 1,
            ram: Some(Ram {
                lines: &ram,
                speed: 104.0,
                park: Point2::from_mm(43.35, 41.85),
            }),
            enter: &enter,
            tower: &tower,
            ..one_layer(&loops, &[0..1], &[])[0]
        },
    ];
    let mut out = Vec::new();
    write_tools(&mut out, &plan, &settings).unwrap();
    let text = String::from_utf8(out).unwrap();
    let at = |wanted: &str| {
        text.lines()
            .position(|line| line.starts_with(wanted))
            .unwrap_or_else(|| panic!("no line starting {wanted}"))
    };
    let change = at("; Change Tool0 -> Tool1");
    // The ram comes before the change, so the old tool lays it: at its
    // own width and speed, and with as much plastic as the line holds.
    let laid = at("G1 X43.350 Y40.850 E");
    assert!(laid < change);
    let rammed = text.lines().nth(laid).unwrap();
    assert_eq!(word(rammed, 'F'), Some(104.0 * 60.0));
    let volume = 2.6 * 0.2 * (1.0 - 0.2 * (1.0 - std::f64::consts::FRAC_PI_4));
    let area = std::f64::consts::PI * 1.75 * 1.75 / 4.0;
    assert!(
        (word(rammed, 'E').unwrap() - volume / area).abs() < 1e-4,
        "{rammed}"
    );
    assert!(text.lines().take(laid).any(|line| line == ";WIDTH:1.000"));
    // The tool then moves clear of it, slowly, and is put away there.
    let parked = at("G1 X43.350 Y41.850");
    assert!(laid < parked && parked < change);
    assert_eq!(word(text.lines().nth(parked).unwrap(), 'F'), Some(2400.0));
    // The new tool comes in from outside the tower's wall, through the
    // gap, and starts where the ram stopped.
    let outside = at("G1 X38.500 Y41.450");
    let inside = at("G1 X40.250 Y41.450");
    let first = at("G1 X60.000 Y41.100 E");
    assert!(change < outside && outside < inside && inside < first);
    // The old tool's count has the ram in it.
    let (_, summary) = (0, write_tools(&mut Vec::new(), &plan, &settings).unwrap());
    assert!(summary.filament_mm3 > volume);

    // With nothing rammed the old tool goes to where the new one starts.
    let plain = [
        plan[0],
        Layer {
            ram: None,
            ..plan[1]
        },
    ];
    let mut out = Vec::new();
    write_tools(&mut out, &plain, &settings).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("G1 X43.350 Y40.850 E") && !text.contains(";WIDTH:1.000"));
}

/// A 10 mm plate with a 20 mm wall on the layer above it: the wall is over
/// the plate along two sides near the corner at the origin and over air
/// along the rest.
fn wall_over_a_smaller_plate(settings: &PrintSettings, measured: bool) -> String {
    let plate = [wall(0.0, 0.0, 10.0, 0)];
    // The wall's middle is 0.225 mm inside the part's edge.
    let above = [wall(0.225, 0.225, 19.55, 0)];
    let beneath = [ExPolygon {
        contour: wall(0.0, 0.0, 10.0, 0).polygon,
        holes: Vec::new(),
    }];
    let layer = |z: f64, loops, below| Layer {
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops,
        islands: &[0..1],
        paths: &[],
        shelter: &[],
        below,
    };
    let layers = [
        layer(0.2, &plate[..], &[][..]),
        layer(
            0.4,
            &above[..],
            if measured { &beneath[..] } else { &[][..] },
        ),
    ];
    let (text, _) = print(&layers, settings);
    // The second layer, without the end of the print, which stops the fan.
    let layer = text.split(";Z:0.400").nth(1).unwrap();
    layer.split("M106 S0").next().unwrap().to_string()
}

fn overhanging() -> PrintSettings {
    PrintSettings {
        outer_wall_speed: 200.0,
        bridge_speed: 25.0,
        bridge_flow: 0.8,
        overhang_speeds: [
            FloatOrPercent::Value(0.0),
            FloatOrPercent::Value(50.0),
            FloatOrPercent::Value(30.0),
            FloatOrPercent::Value(10.0),
        ],
        overhang_fan: Some(OverhangFan {
            speed: 100.0,
            at: Some(0.5),
        }),
        // A layer fan that is the same however long the layer takes.
        fan_min_speed: 20.0,
        fan_max_speed: 20.0,
        reduce_fan_stop_start_freq: true,
        close_fan_the_first_x_layers: 0,
        full_fan_speed_layer: 0,
        ..basic()
    }
}

#[test]
fn a_wall_over_air_is_bridged_and_cooled() {
    let text = wall_over_a_smaller_plate(&overhanging(), true);
    // It starts over the plate as the outer wall the layer below ended
    // with, so that kind is not named again until it comes back to it.
    assert_eq!(features(&text), ["Overhang wall", "Outer wall"]);
    assert!(!text.contains(";_FAN"), "no marker is left in the output");

    // The fan goes to full for the stretch over air and back to the
    // layer's 20% after it.
    let fans: Vec<&str> = text
        .lines()
        .filter(|line| line.starts_with("M106"))
        .collect();
    assert_eq!(fans, ["M106 S255", "M106 S51"]);
    let at = |needle: &str| text.find(needle).unwrap();
    assert!(at("M106 S255") < at(";TYPE:Overhang wall"));
    assert!(at(";TYPE:Overhang wall") < at("M106 S51"));

    // Over the plate the wall goes at its own speed, over air at the
    // bridge's, with four fifths of the plastic.
    let mut kind = "";
    let (mut over_plate, mut over_air) = (Vec::new(), Vec::new());
    let (mut speed, mut at_point) = (0.0, None::<(f64, f64)>);
    for line in text.lines() {
        if let Some(name) = line.strip_prefix(";TYPE:") {
            kind = name;
        }
        if !line.starts_with("G1 ") {
            continue;
        }
        speed = word(line, 'F').unwrap_or(speed);
        let to = word(line, 'X').zip(word(line, 'Y'));
        if let (Some(feed), Some(from), Some(to)) = (word(line, 'E'), at_point, to)
            && feed > 0.0
        {
            let length = (to.0 - from.0).hypot(to.1 - from.1);
            let entry = (speed / 60.0, feed / length);
            if kind == "Overhang wall" {
                over_air.push(entry);
            } else {
                over_plate.push(entry);
            }
        }
        at_point = to.or(at_point);
    }
    assert!(over_air.iter().all(|(speed, _)| *speed <= 25.0));
    assert!(over_plate.iter().any(|(speed, _)| *speed == 200.0));
    let (plain, bridged) = (over_plate[0].1, over_air[0].1);
    assert!(
        (bridged / plain - 0.8).abs() < 1e-3,
        "{bridged} against {plain}"
    );
    // Between the two, where the wall is only part supported, it slows.
    assert!(
        over_plate
            .iter()
            .any(|(speed, _)| *speed < 200.0 && *speed >= 10.0),
        "{over_plate:?}"
    );
}

#[test]
fn overhangs_are_left_alone_when_asked_or_when_nothing_is_known() {
    // With nothing known of the layer beneath, the wall is as it was.
    let unmeasured = wall_over_a_smaller_plate(&overhanging(), false);
    assert!(features(&unmeasured).is_empty());
    assert!(!unmeasured.contains("M106"));

    // Every part of it switched off.
    let off = PrintSettings {
        detect_overhang_wall: false,
        enable_overhang_speed: false,
        overhang_fan: None,
        ..overhanging()
    };
    assert_eq!(wall_over_a_smaller_plate(&off, true), unmeasured);

    // Slowed but not bridged: one kind of line, more than one speed.
    let slowed = PrintSettings {
        detect_overhang_wall: false,
        overhang_fan: None,
        ..overhanging()
    };
    let text = wall_over_a_smaller_plate(&slowed, true);
    assert!(features(&text).is_empty());
    assert!(
        text.contains("F1500"),
        "over air it goes at the bridge's speed"
    );

    // A layer fan already at the overhang's speed is not touched.
    let windy = PrintSettings {
        fan_min_speed: 100.0,
        fan_max_speed: 100.0,
        ..overhanging()
    };
    let text = wall_over_a_smaller_plate(&windy, true);
    assert!(!text.contains("M106") && !text.contains(";_FAN"));
    assert!(text.contains(";TYPE:Overhang wall"));
}

#[test]
fn solid_over_infill_goes_at_its_own_bridge_speed() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::InternalBridge)];
    let settings = PrintSettings {
        bridge_speed: 40.0,
        internal_bridge_speed: FloatOrPercent::Percent(150.0),
        ..basic()
    };
    let second = |settings: &PrintSettings| {
        let layer = |z: f64| Layer {
            z,
            ..one_layer(&loops, &[0..1], &fill)[0]
        };
        let (text, _) = print(&[layer(0.2), layer(0.4)], settings);
        text.split(";Z:0.400").nth(1).unwrap().to_string()
    };
    let text = second(&settings);
    assert_eq!(features(&text), ["Outer wall", "Internal Bridge"]);
    let bridge = text.split(";TYPE:Internal Bridge").nth(1).unwrap();
    let speed = moves(bridge).find_map(|line| word(line, 'F')).unwrap();
    assert_eq!(speed, 60.0 * 60.0);
    // A speed given outright is used as it is.
    let fixed = PrintSettings {
        internal_bridge_speed: FloatOrPercent::Value(70.0),
        ..settings
    };
    let text = second(&fixed);
    let bridge = text.split(";TYPE:Internal Bridge").nth(1).unwrap();
    assert_eq!(
        moves(bridge).find_map(|line| word(line, 'F')),
        Some(70.0 * 60.0)
    );
}

#[test]
fn the_file_is_laid_out_as_printers_expect() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let layer = |z: f64, filament: usize| Layer {
        z,
        filament,
        ..one_layer(&loops, &[0..1], &[])[0]
    };
    let settings = |density: f64| PrintSettings {
        filament_density: Some(density),
        nozzle_temperature_initial_layer: 215.0,
        bed_temperature_initial_layer: Some(65.0),
        config: vec![
            ("filament_density".into(), "1.24,1.27".into()),
            ("filament_type".into(), "PLA;PETG".into()),
            ("printable_area".into(), "0x0,270x0,270x270,0x270".into()),
        ],
        produced_at: Some("2026-11-10 at 07:21:37".into()),
        ..basic()
    };
    let mut out = Vec::new();
    write_tools(
        &mut out,
        &[layer(0.2, 0), layer(0.4, 0), layer(0.4, 1)],
        &[settings(1.24), settings(1.27)],
    )
    .unwrap();
    let text = String::from_utf8(out).unwrap();

    // The U1's host takes a file only from a slicer it knows by name.
    let produced = format!(
        "; generated by {} (EdgeSlicerRS {}) on 2026-11-10 at 07:21:37",
        edge_config::reference_producer(),
        env!("CARGO_PKG_VERSION")
    );
    assert!(produced.starts_with("; generated by OrcaSlicer "));
    let head: Vec<&str> = text.lines().take(9).collect();
    assert_eq!(
        head,
        [
            "; HEADER_BLOCK_START",
            produced.as_str(),
            "; total layer number: 2",
            "; filament_density: 1.24,1.27",
            "; max_z_height: 0.40",
            "; filament: 1,2",
            "; support_material_on_wipe_tower: 0",
            "; HEADER_BLOCK_END",
            "",
        ]
    );
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("no {needle:?} in the file"))
    };
    // Everything the printer runs lies between the two markers.
    assert!(at("; EXECUTABLE_BLOCK_START") < at("G1 "));
    assert!(text.rfind("G1 ").unwrap() < at("; EXECUTABLE_BLOCK_END"));
    // After it: what each filament used, the settings, and the totals.
    let tail = &text[at("; EXECUTABLE_BLOCK_END")..];
    let line = |start: &str| {
        tail.lines()
            .find(|line| line.starts_with(start))
            .unwrap_or_else(|| panic!("no line starting {start:?}"))
    };
    let used: Vec<f64> = line("; filament used [mm] = ")["; filament used [mm] = ".len()..]
        .split(", ")
        .map(|length| length.parse().unwrap())
        .collect();
    assert_eq!(used.len(), 2);
    assert!((used[0] - 2.0 * used[1]).abs() < 0.02, "{used:?}");
    assert!(line("; filament used [g] = ").contains(", "));
    let order = [
        "; CONFIG_BLOCK_START",
        "; filament_type = PLA;PETG",
        "; first_layer_bed_temperature = 65",
        "; bed_shape = 0x0,270x0,270x270,0x270",
        "; first_layer_temperature = 215",
        "; first_layer_height = 0.200",
        "; CONFIG_BLOCK_END",
        "; total filament used [g] = ",
        "; total filament change = 1",
        "; total layers count = 2",
        "; estimated printing time (normal mode) = ",
        "; estimated first layer printing time (normal mode) = ",
    ];
    let places: Vec<usize> = order.iter().map(|needle| at(needle)).collect();
    assert!(
        places.windows(2).all(|pair| pair[0] < pair[1]),
        "{places:?}"
    );

    // With no time given, none is made up.
    let (text, _) = print(&one_layer(&loops, &[0..1], &[]), &basic());
    let first = text.lines().nth(1).unwrap();
    assert!(first.ends_with(')') && !first.contains(" on "), "{first}");
    // Nor is a printer with no profile told its progress.
    assert!(!text.contains("M73"));
    assert!(!text.contains("CONFIG_BLOCK"));
}

#[test]
fn times_are_written_as_the_reference_writes_them() {
    assert_eq!(days_hours_minutes_seconds(19.4), "19s");
    assert_eq!(days_hours_minutes_seconds(632.0), "10m 32s");
    assert_eq!(days_hours_minutes_seconds(3_723.0), "1h 2m 3s");
    assert_eq!(days_hours_minutes_seconds(90_061.0), "1d 1h 1m 1s");
}

#[test]
fn progress_is_said_at_the_start_and_as_layers_go_by() {
    // Ten layers, each a 40 mm wall at 2 mm/s: twenty seconds a layer
    // and more, so the print is some minutes long.
    let settings = PrintSettings {
        outer_wall_speed: 2.0,
        initial_layer_speed: 2.0,
        ..profile(&[("start", "PRINT_START"), ("end", "PRINT_END")])
    };
    let text = printed(10, &settings);
    let said: Vec<(u32, u32)> = text
        .lines()
        .filter_map(|line| line.strip_prefix("M73 P"))
        .map(|rest| {
            let (percent, minutes) = rest.split_once(" R").unwrap();
            (percent.parse().unwrap(), minutes.parse().unwrap())
        })
        .collect();
    assert!(said[0].0 == 0 && said[0].1 >= 3, "{said:?}");
    assert_eq!(said.last(), Some(&(100, 0)));
    assert!(said.len() >= 10, "{said:?}");
    assert!(
        said.windows(2)
            .all(|pair| pair[1].0 >= pair[0].0 && pair[1].1 <= pair[0].1),
        "{said:?}"
    );
    // The first comes before anything the printer is told to do, and
    // every other one before a layer.
    let lines: Vec<&str> = text.lines().collect();
    let first = lines
        .iter()
        .position(|line| line.starts_with("M73"))
        .unwrap();
    assert_eq!(lines[first - 1], "; EXECUTABLE_BLOCK_START");
    assert_eq!(
        lines[lines.len()
            - 1
            - lines
                .iter()
                .rev()
                .position(|line| line.starts_with("M73"))
                .unwrap()
            + 1],
        "; EXECUTABLE_BLOCK_END"
    );
}

#[test]
fn a_wall_with_nothing_under_it_follows_the_curled_edge_setting() {
    // As the profile has it, a wall over air goes at the bridge's speed.
    let at_bridge_speed = wall_over_a_smaller_plate(&overhanging(), true);
    let over_air = |text: &str| -> Vec<f64> {
        let mut speeds = Vec::new();
        let (mut kind, mut speed) = ("", 0.0);
        for line in text.lines() {
            if let Some(name) = line.strip_prefix(";TYPE:") {
                kind = name;
            }
            if line.starts_with("G1 ") {
                speed = word(line, 'F').unwrap_or(speed);
                if kind == "Overhang wall" && word(line, 'E').is_some_and(|feed| feed > 0.0) {
                    speeds.push(speed / 60.0);
                }
            }
        }
        speeds
    };
    let speeds = over_air(&at_bridge_speed);
    assert!(speeds.contains(&25.0), "{speeds:?}");
    // With the setting on, at the last of the overhang speeds.
    let careful = PrintSettings {
        slowdown_for_curled_perimeters: true,
        ..overhanging()
    };
    let speeds = over_air(&wall_over_a_smaller_plate(&careful, true));
    assert!(
        !speeds.is_empty() && speeds.iter().all(|speed| *speed <= 10.0),
        "{speeds:?}"
    );
}

#[test]
fn a_wall_over_air_is_as_wide_as_an_inner_wall() {
    // An outer wall 0.42 mm wide with an inner wall of 0.45 mm behind
    // it, over a plate half their size.
    let narrow = Flow::new(0.42, 0.2, 0.4).unwrap();
    let plate = [wall(0.0, 0.0, 10.0, 0)];
    let above = [
        ExtrusionLoop {
            flow: narrow,
            ..wall(0.21, 0.21, 19.58, 0)
        },
        wall(0.64, 0.64, 18.72, 1),
    ];
    let beneath = [ExPolygon {
        contour: wall(0.0, 0.0, 10.0, 0).polygon,
        holes: Vec::new(),
    }];
    let layers = [
        Layer {
            z: 0.2,
            ..one_layer(&plate, &[0..1], &[])[0]
        },
        Layer {
            z: 0.4,
            below: &beneath,
            ..one_layer(&above, &[0..2], &[])[0]
        },
    ];
    let (text, _) = print(&layers, &overhanging());
    let second = text.split(";Z:0.400").nth(1).unwrap();
    // Every stretch over air is written at the inner wall's width less
    // what the bridge's flow takes off, whichever wall it belongs to.
    let wide = flow().bridging(0.8, false).width;
    let mut kind = "";
    let mut widths = Vec::new();
    for line in second.lines() {
        if let Some(name) = line.strip_prefix(";TYPE:") {
            kind = name;
        }
        if let Some(width) = line.strip_prefix(";WIDTH:")
            && kind == "Overhang wall"
        {
            widths.push(width.parse::<f64>().unwrap());
        }
    }
    assert!(
        widths.iter().any(|width| (width - wide).abs() < 1e-3),
        "{widths:?} against {wide}"
    );
    assert!(
        !widths
            .iter()
            .any(|width| (width - narrow.bridging(0.8, false).width).abs() < 1e-3),
        "{widths:?}"
    );
    // With a width set for bridges (`bridge_line_width`) it is that
    // width the bridge's flow thins.
    let sized = PrintSettings {
        bridge_line_width: Some(0.3),
        ..overhanging()
    };
    let (text, _) = print(&layers, &sized);
    let second = text.split(";Z:0.400").nth(1).unwrap();
    let set = flow().with_width(0.3).unwrap().bridging(0.8, false).width;
    let mut kind = "";
    let mut widths = Vec::new();
    for line in second.lines() {
        if let Some(name) = line.strip_prefix(";TYPE:") {
            kind = name;
        }
        if let Some(width) = line.strip_prefix(";WIDTH:")
            && kind == "Overhang wall"
        {
            widths.push(width.parse::<f64>().unwrap());
        }
    }
    assert!(
        !widths.is_empty() && widths.iter().all(|width| (width - set).abs() < 1e-3),
        "{widths:?} against {set}"
    );
}

#[test]
fn some_kinds_of_line_have_a_share_of_their_own() {
    let fill = [
        line((0.0, 0.0), (10.0, 0.0), Role::SolidInfill),
        line((0.0, 1.0), (10.0, 1.0), Role::InternalBridge),
        line((0.0, 2.0), (10.0, 2.0), Role::TopSolidInfill),
        line((0.0, 3.0), (10.0, 3.0), Role::BottomSurface),
    ];
    let loops = [ExtrusionLoop {
        role: Role::Brim,
        ..wall(20.0, 0.0, 10.0, 0)
    }];
    let settings = PrintSettings {
        internal_bridge_flow: 0.95,
        top_solid_infill_flow_ratio: 0.9,
        bottom_solid_infill_flow_ratio: 1.1,
        brim_flow_ratio: 0.5,
        ..basic()
    };
    let layer = Layer {
        around: &loops,
        ..one_layer(&[], &[], &fill)[0]
    };
    let (text, _) = print(&[layer], &settings);
    // Filament fed over each kind's first 10 mm move.
    let fed = |kind: &str| -> f64 {
        let after = text.split(&format!(";TYPE:{kind}\n")).nth(1).unwrap();
        moves(after)
            .find_map(|line| word(line, 'E').filter(|feed| *feed > 0.0))
            .unwrap()
    };
    let plain = fed("Internal solid infill");
    for (kind, share) in [
        ("Internal Bridge", 0.95),
        ("Top surface", 0.9),
        ("Bottom surface", 1.1),
        ("Brim", 0.5),
    ] {
        assert!(
            (fed(kind) / plain - share).abs() < 1e-3,
            "{kind}: {} against {plain}",
            fed(kind)
        );
    }
}

#[test]
fn a_print_with_a_printer_profile_is_timed_as_the_printer_runs_it() {
    // A 120 mm square loop, timed first at its speed alone.
    let loops = [wall(0.0, 0.0, 30.0, 0)];
    let plain = PrintSettings {
        slow_down_for_layer_cooling: false,
        ..basic()
    };
    let (text, summary) = print(&one_layer(&loops, &[0..1], &[]), &plain);
    assert_eq!(summary.timed, None);
    assert_eq!(summary.seconds, summary.seconds_at_speeds);
    assert!(!text.contains("time planned"));

    // On a Klipper printer at 1000 mm/s² every corner costs time: the
    // nozzle slows to the square corner velocity and speeds up again.
    let limits = edge_time::Limits {
        flavor: edge_time::Flavor::Klipper,
        max_extruding: 1000.0,
        jerk: [5.0, 5.0, 0.0, 0.0],
        ..edge_time::Limits::default()
    };
    let known = PrintSettings {
        limits: Some(limits),
        ..plain.clone()
    };
    let (text, timed) = print(&one_layer(&loops, &[0..1], &[]), &known);
    assert_eq!(timed.timed, Some(TimeModel::Klipper));
    assert_eq!(timed.seconds_at_speeds, summary.seconds_at_speeds);
    assert!(
        timed.seconds > timed.seconds_at_speeds + 0.1,
        "{} against {}",
        timed.seconds,
        timed.seconds_at_speeds
    );
    // The file says the same, and how it was come by.
    let stated = edge_time::estimate(&text, &limits, TimeModel::Klipper).seconds;
    assert!((stated - timed.seconds).abs() < 1e-9);
    assert!(text.contains("; time planned as Klipper plans"));
    assert!(text.contains("; time at commanded speeds, without acceleration [s] = "));
}

#[test]
fn a_wall_whose_width_changes_is_laid_from_its_seam_in_pieces_end_to_end() {
    // A 10 mm square, 0.40 mm wide along the bottom and 0.60 along the top.
    let mut varying = wall(0.0, 0.0, 10.0, 0);
    varying.widths = vec![400_000, 400_000, 600_000, 600_000];
    let plain = [wall(0.0, 0.0, 10.0, 0)];
    let loops = [varying];
    let (text, _) = print(&one_layer(&loops, &[0..1], &[]), &basic());
    let (even, _) = print(&one_layer(&plain, &[0..1], &[]), &basic());

    // Once round, with no move between the pieces that lays nothing.
    let laid: Vec<&str> = moves(&text)
        .filter(|line| word(line, 'X').is_some() || word(line, 'Y').is_some())
        .collect();
    let first = laid
        .iter()
        .position(|line| word(line, 'E').is_some())
        .unwrap();
    assert!(laid[first..].iter().all(|line| word(line, 'E').is_some()));
    // The widths are told as they change.
    let widths: Vec<f64> = text
        .lines()
        .filter_map(|line| line.strip_prefix(";WIDTH:")?.parse().ok())
        .collect();
    assert!(widths.len() >= 4, "{widths:?}");
    let (least, most) = (
        widths.iter().copied().fold(f64::INFINITY, f64::min),
        widths.iter().copied().fold(0.0, f64::max),
    );
    // A width is what the generator gave plus what a line's rounded
    // edges leave empty: 0.2 mm * (1 - pi/4).
    assert!(
        (least - 0.443).abs() < 0.01 && (most - 0.643).abs() < 0.01,
        "{least}..{most}"
    );

    // Plastic per mm follows the width: along the bottom less than along the top.
    let per_mm = |y: f64| -> f64 {
        let mut at = (0.0, 0.0);
        let mut found = 0.0;
        for line in moves(&text) {
            let to = (
                word(line, 'X').unwrap_or(at.0),
                word(line, 'Y').unwrap_or(at.1),
            );
            if let Some(e) = word(line, 'E')
                && at.1 == y
                && to.1 == y
                && to.0 != at.0
            {
                found = e / (to.0 - at.0).abs();
            }
            at = to;
        }
        found
    };
    assert!(per_mm(0.0) > 0.0 && per_mm(10.0) > 1.3 * per_mm(0.0));

    // As much plastic as the same wall at the width between.
    let fed = |text: &str| -> f64 {
        moves(text)
            .filter_map(|line| word(line, 'E'))
            .filter(|e| *e > 0.0)
            .sum()
    };
    let between = {
        let mut middle = wall(0.0, 0.0, 10.0, 0);
        middle.flow = flow()
            .with_width(0.5 + 0.2 * (1.0 - std::f64::consts::FRAC_PI_4))
            .unwrap();
        print(&one_layer(&[middle], &[0..1], &[]), &basic()).0
    };
    assert!(
        (fed(&text) / fed(&between) - 1.0).abs() < 0.02,
        "{} against {}",
        fed(&text),
        fed(&between)
    );
    assert!(fed(&even) > 0.0);
}

#[test]
fn widths_follow_a_wall_opened_at_its_seam_and_cut_short() {
    let square =
        [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)].map(|(x, y)| Point2::from_mm(x, y));
    let widths = [400.0, 500.0, 600.0, 700.0];
    // Opened half way along the first edge: the width there is half way too.
    let start = Point2::from_mm(5.0, 0.0);
    let path = seam::open_at(&square, 0, start);
    let opened = seam::open_widths_at(&square, &widths, 0, start);
    assert_eq!(path.len(), opened.len());
    assert_eq!(opened, [450.0, 500.0, 600.0, 700.0, 400.0, 450.0]);
    // Opened at a corner, the widths are only turned.
    let at_corner = seam::open_widths_at(&square, &widths, 2, square[2]);
    assert_eq!(at_corner, [600.0, 700.0, 400.0, 500.0, 600.0]);
    // Cut 2.5 mm short of its end: the last width is that of where it stops.
    let mut clipped = path.clone();
    seam::clip_end(&mut clipped, 2.5);
    let cut = seam::clip_widths(&path, &opened, &clipped);
    assert_eq!(cut.len(), clipped.len());
    assert_eq!(cut[cut.len() - 1], 425.0);
    // Cut by more than the last piece: the points left keep their widths.
    let mut shorter = path.clone();
    seam::clip_end(&mut shorter, 10.0);
    let cut = seam::clip_widths(&path, &opened, &shorter);
    assert_eq!(cut.len(), shorter.len());
    assert_eq!(cut[..3], opened[..3]);
}
