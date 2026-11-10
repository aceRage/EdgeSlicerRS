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
        loops,
        islands,
        paths,
        infill_area: &[],
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
        ..PrintSettings::default()
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
    let (text, _) = print(&one_layer(&loops, &[0..1], &[]), &PrintSettings::default());
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
    let settings = PrintSettings::default();
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
    assert!(!text.contains("Z0.600"));

    let none = PrintSettings {
        retraction_length: 0.0,
        ..PrintSettings::default()
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
    let (text, _) = print(&one_layer(&loops, &islands, &[]), &PrintSettings::default());
    assert_eq!(
        features(&text),
        ["Inner wall", "Outer wall", "Inner wall", "Outer wall"]
    );

    let outer_first = PrintSettings {
        wall_order: WallOrder::OuterInner,
        ..PrintSettings::default()
    };
    let (text, _) = print(&one_layer(&loops, &islands, &[]), &outer_first);
    assert_eq!(
        features(&text),
        ["Outer wall", "Inner wall", "Outer wall", "Inner wall"]
    );

    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SolidInfill)];
    let infill_first = PrintSettings {
        infill_first: true,
        ..PrintSettings::default()
    };
    let (text, _) = print(&one_layer(&loops[..2], &[0..2], &fill), &infill_first);
    assert_eq!(
        features(&text),
        ["Internal solid infill", "Inner wall", "Outer wall"]
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
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            infill_area: &[],
        },
        Layer {
            z: 0.4,
            height: 0.2,
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            infill_area: &[],
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
    let (text, _) = print(&layers, &PrintSettings::default());
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
        ..PrintSettings::default()
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
        loops: &loops,
        islands: &[0..1],
        paths: &[],
        infill_area: &[],
    };
    let settings = PrintSettings {
        nozzle_temperature: 210.0,
        nozzle_temperature_initial_layer: 220.0,
        bed_temperature: Some(55.0),
        bed_temperature_initial_layer: Some(60.0),
        filament_density: Some(1.24),
        ..PrintSettings::default()
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
            "M83",
            "M104 S210",
            "M140 S55",
            "M104 S0",
            "M140 S0",
            "M84"
        ]
    );
    let grams = summary.filament_mm3 / 1000.0 * 1.24;
    assert!(text.contains(&format!("; filament used [g] = {grams:.2}")));

    // With no bed type known, the bed is left alone.
    let (text, _) = print(&[layer(0.2)], &PrintSettings::default());
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
    let (text, summary) = print(&one_layer(&[], &[], &fill), &PrintSettings::default());
    let rows: Vec<f64> = moves(&text)
        .filter(|l| l.contains(" E"))
        .filter_map(|line| word(line, 'Y'))
        .collect();
    assert_eq!(rows, [0.0, 1.0, 2.0]);
    // Each row starts where the last one ended: two 1 mm hops, no retraction but the last.
    assert!((summary.travel_mm - 2.0).abs() < 1e-9);
    assert_eq!(summary.retractions, 1);
    assert_eq!(
        print(&one_layer(&[], &[], &fill), &PrintSettings::default()).0,
        text
    );
}

#[test]
fn moves_inside_the_infill_area_can_skip_retraction() {
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
        loops: &[],
        islands: &[],
        paths: &fill,
        infill_area: &area,
    }];

    let plain = PrintSettings::default();
    assert_eq!(print(&layer, &plain).1.retractions, 2);
    let reduced = PrintSettings {
        reduce_infill_retraction: true,
        ..PrintSettings::default()
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
        infill_area: &holed,
        ..layer[0]
    }];
    assert_eq!(print(&layer, &reduced).1.retractions, 2);
}
