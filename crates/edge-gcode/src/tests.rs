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
        around: &[],
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

#[test]
fn speeds_follow_role_layer_and_the_melt_limit() {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SparseInfill)];
    let layers = [
        Layer {
            z: 0.2,
            height: 0.2,
            around: &[],
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            infill_area: &[],
        },
        Layer {
            z: 0.4,
            height: 0.2,
            around: &[],
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
        around: &[],
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
        around: &[],
        loops: &[],
        islands: &[],
        paths: &fill,
        infill_area: &area,
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
        infill_area: &holed,
        ..layer[0]
    }];
    assert_eq!(print(&layer, &reduced).1.retractions, 2);
}

/// A wall and one fill line on each of `count` layers, 0.2 mm apart.
fn printed(count: usize, settings: &PrintSettings) -> String {
    let loops = [wall(0.0, 0.0, 10.0, 0)];
    let fill = [line((2.0, 2.0), (8.0, 2.0), Role::SparseInfill)];
    let layers: Vec<Layer<'_>> = (0..count)
        .map(|index| Layer {
            z: 0.2 * (index + 1) as f64,
            height: 0.2,
            around: &[],
            loops: &loops,
            islands: &[0..1],
            paths: &fill,
            infill_area: &[],
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
    let start: Vec<&str> = text
        .lines()
        .take_while(|line| *line != ";LAYER_CHANGE")
        .collect();
    assert_eq!(
        start[1..],
        [
            ";TYPE:Custom",
            "PRINT_START TOTAL=3",
            "G1 X185 E15 F360",
            "G90",
            "G21",
            "M83 ; use relative distances for extrusion",
            "; filament start",
            "T0",
            "M106 S0",
        ]
    );
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
        .rev()
        .skip_while(|line| line.starts_with("; "))
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
            "SET_VELOCITY_LIMIT ACCEL=500 ACCEL_TO_DECEL=250",
            // Second layer: travel, the outer wall, then fill at the default.
            "SET_VELOCITY_LIMIT ACCEL=10000 ACCEL_TO_DECEL=5000",
            "SET_VELOCITY_LIMIT ACCEL=5000 ACCEL_TO_DECEL=2500",
            "SET_VELOCITY_LIMIT ACCEL=10000 ACCEL_TO_DECEL=5000",
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
