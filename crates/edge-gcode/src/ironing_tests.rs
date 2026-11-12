//! How ironing is written: under its own name, at its own speed, last
//! in its island, and with its own fan speed where it has one.

// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use edge_config::Settings;
use edge_geom::Polygon;

use super::*;

/// The U1's ironing line: 0.0075 mm tall, the nozzle's bore between
/// its rounded edges.
fn ironing_flow() -> Flow {
    Flow {
        width: 0.4 + 0.0075 * (1.0 - 0.25 * PI),
        height: 0.0075,
        spacing: 0.4,
        nozzle_diameter: 0.4,
    }
}

fn flow() -> Flow {
    Flow::new(0.45, 0.2, 0.4).unwrap()
}

/// A square outer wall with its lower-left corner at `(x, y)`.
fn wall(x: f64, y: f64, side: f64) -> ExtrusionLoop {
    ExtrusionLoop {
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
        polygon: Polygon {
            points: [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        },
        role: Role::ExternalPerimeter,
        is_hole: false,
        depth: 0,
        flow: flow(),
    }
}

fn line(a: (f64, f64), b: (f64, f64), role: Role, part: u32) -> ExtrusionPath {
    ExtrusionPath {
        points: vec![Point2::from_mm(a.0, a.1), Point2::from_mm(b.0, b.1)],
        role,
        flow: if role == Role::Ironing {
            ironing_flow()
        } else {
            flow()
        },
        in_turn: false,
        part,
    }
}

/// The reference's defaults, less what gets in the way of reading a
/// small test, with ironing at a speed nothing else has.
fn basic() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        seam_gap: 0.0,
        lift: Lift::Normal,
        slow_down_for_layer_cooling: false,
        ironing_speed: 17.0,
        ..PrintSettings::default()
    }
}

fn layer<'a>(
    z: f64,
    loops: &'a [ExtrusionLoop],
    islands: &'a [Range<usize>],
    paths: &'a [ExtrusionPath],
) -> Layer<'a> {
    Layer {
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
        loops,
        islands,
        paths,
        shelter: &[],
        below: &[],
    }
}

fn print(layers: &[Layer<'_>], settings: &PrintSettings) -> String {
    let mut out = Vec::new();
    write(&mut out, layers, settings).unwrap();
    String::from_utf8(out).unwrap()
}

fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

fn features(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| line.strip_prefix(";TYPE:"))
        .collect()
}

/// The text from a layer's start to the next layer's, or the end.
fn layer_text(text: &str, index: usize) -> &str {
    let starts: Vec<usize> = text
        .match_indices(";LAYER_CHANGE")
        .map(|(at, _)| at)
        .collect();
    let end = starts.get(index + 1).copied().unwrap_or(text.len());
    &text[starts[index]..end]
}

/// The lines written for one feature on a layer: from its name to the
/// next name.
fn feature_lines<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    text.lines()
        .skip_while(|line| line.strip_prefix(";TYPE:") != Some(name))
        .skip(1)
        .take_while(|line| !line.starts_with(";TYPE:"))
        .collect()
}

/// The feedrate ironing moves are made at, in mm/s: the last set before
/// the first of them.
fn ironing_speed(text: &str) -> f64 {
    let mut feed = 0.0;
    let mut ironing = false;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix(";TYPE:") {
            ironing = name == "Ironing";
        }
        if let Some(f) = word(line, 'F') {
            feed = f;
        }
        if ironing && word(line, 'E').is_some_and(|e| e > 0.0) && word(line, 'X').is_some() {
            return feed / 60.0;
        }
    }
    panic!("no ironing move in:\n{text}");
}

/// Two layers of a 10 mm square, the second with a top surface line
/// and an ironing line over it.
fn ironed_square(settings: &PrintSettings, ironing: &[ExtrusionPath]) -> String {
    let loops = [wall(0.0, 0.0, 10.0)];
    let mut fill = vec![line((1.0, 5.0), (9.0, 5.0), Role::TopSolidInfill, 1)];
    fill.extend(ironing.iter().cloned());
    let plain = [line((1.0, 5.0), (9.0, 5.0), Role::SolidInfill, 1)];
    print(
        &[
            layer(0.2, &loops, &[0..1], &plain),
            layer(0.4, &loops, &[0..1], &fill),
        ],
        settings,
    )
}

#[test]
fn ironing_is_written_under_its_name_with_its_own_line() {
    let ironing = [line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2)];
    let text = ironed_square(&basic(), &ironing);
    assert_eq!(role_name(Role::Ironing), "Ironing");
    let second = layer_text(&text, 1);
    assert_eq!(features(second), ["Outer wall", "Top surface", "Ironing"]);
    let lines = feature_lines(second, "Ironing");
    assert!(lines.contains(&";WIDTH:0.402"), "{lines:?}");
    // 8 mm of a line holding 0.4 x 0.0075 mm² of plastic, through
    // 1.75 mm filament: no share of its own is taken off or added.
    let fed: f64 = lines
        .iter()
        .filter(|line| word(line, 'X').is_some())
        .filter_map(|line| word(line, 'E'))
        .sum();
    let expected = 8.0 * 0.4 * 0.0075 / (PI * 0.875 * 0.875);
    assert!((fed - expected).abs() < 1e-5, "{fed} against {expected}");
}

#[test]
fn ironing_goes_at_its_own_speed_and_the_default_acceleration() {
    let ironing = [line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2)];
    let settings = PrintSettings {
        default_acceleration: 4321.0,
        top_surface_acceleration: 2000.0,
        klipper: false,
        ..basic()
    };
    let text = ironed_square(&settings, &ironing);
    let second = layer_text(&text, 1);
    assert_eq!(ironing_speed(second), 17.0);
    // The last acceleration said before the ironing is the default,
    // not the top surface's it follows.
    let before: Vec<&str> = second
        .lines()
        .take_while(|line| *line != ";TYPE:Ironing")
        .chain(feature_lines(second, "Ironing"))
        .filter(|line| line.starts_with("M204"))
        .collect();
    assert_eq!(before.last(), Some(&"M204 S4321"), "{before:?}");
    // The hot end's limit holds no ironing back: there is next to no
    // plastic in it.
    let throttled = PrintSettings {
        filament_max_volumetric_speed: 1.0,
        ..basic()
    };
    assert_eq!(
        ironing_speed(layer_text(&ironed_square(&throttled, &ironing), 1)),
        17.0
    );
}

#[test]
fn on_the_first_layer_ironing_is_no_faster_than_ironing_or_that_layers_fill() {
    // Not as the reference, which irons a first layer at
    // `initial_layer_infill_speed` whatever the ironing speed is.
    let loops = [wall(0.0, 0.0, 10.0)];
    let fill = [
        line((1.0, 5.0), (9.0, 5.0), Role::BottomSurface, 1),
        line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2),
    ];
    let first = |initial_layer_infill_speed: f64| {
        let settings = PrintSettings {
            initial_layer_infill_speed,
            ..basic()
        };
        ironing_speed(&print(&[layer(0.2, &loops, &[0..1], &fill)], &settings))
    };
    assert_eq!(first(105.0), 17.0);
    assert_eq!(first(10.0), 10.0);
    // A first layer speed of nothing is no speed to hold ironing to.
    assert_eq!(first(0.0), 17.0);
}

#[test]
fn ironing_comes_last_in_its_island_whatever_the_order_of_walls_and_fill() {
    // Given first, to show it is not the order given that puts it last.
    let fill = [
        line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2),
        line((1.0, 5.0), (9.0, 5.0), Role::TopSolidInfill, 1),
    ];
    let loops = [wall(0.0, 0.0, 10.0)];
    let layers = [layer(0.2, &loops, &[0..1], &fill)];
    assert_eq!(
        features(&print(&layers, &basic())),
        ["Outer wall", "Top surface", "Ironing"]
    );
    let infill_first = PrintSettings {
        infill_first: true,
        ..basic()
    };
    assert_eq!(
        features(&print(&layers, &infill_first)),
        ["Top surface", "Outer wall", "Ironing"]
    );
}

#[test]
fn each_island_is_ironed_before_the_next_is_begun() {
    let loops = [wall(0.0, 0.0, 10.0), wall(30.0, 0.0, 10.0)];
    let fill = [
        line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 3),
        line((31.0, 2.0), (39.0, 2.0), Role::Ironing, 4),
        line((1.0, 5.0), (9.0, 5.0), Role::TopSolidInfill, 1),
        line((31.0, 5.0), (39.0, 5.0), Role::TopSolidInfill, 2),
    ];
    let text = print(&[layer(0.2, &loops, &[0..1, 1..2], &fill)], &basic());
    assert_eq!(
        features(&text),
        [
            "Outer wall",
            "Top surface",
            "Ironing",
            "Outer wall",
            "Top surface",
            "Ironing"
        ]
    );
    // The first island's ironing is the one inside it.
    let first: Vec<f64> = feature_lines(&text, "Ironing")
        .iter()
        .filter(|line| word(line, 'E').is_some_and(|e| e > 0.0))
        .filter_map(|line| word(line, 'X'))
        .collect();
    assert!(first.iter().all(|x| *x < 10.0), "{first:?}");

    // Ironing that lies in no island still follows the fill that does not.
    let loose = [
        line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2),
        line((1.0, 5.0), (9.0, 5.0), Role::TopSolidInfill, 1),
    ];
    assert_eq!(
        features(&print(&[layer(0.2, &[], &[], &loose)], &basic())),
        ["Top surface", "Ironing"]
    );
}

#[test]
fn ironing_is_swept_from_the_end_it_was_made_to_start_at() {
    // The top surface ends at x = 9, by the far end of the ironing
    // line, which is nonetheless begun at its first point.
    let fill = [
        line((1.0, 5.0), (9.0, 5.0), Role::TopSolidInfill, 1),
        line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2),
    ];
    let loops = [wall(0.0, 0.0, 10.0)];
    let text = print(&[layer(0.2, &loops, &[0..1], &fill)], &basic());
    let xs: Vec<f64> = feature_lines(&text, "Ironing")
        .iter()
        .filter(|line| word(line, 'E').is_some_and(|e| e > 0.0))
        .filter_map(|line| word(line, 'X'))
        .collect();
    assert_eq!(xs, [9.0]);

    // Several lines of an area keep the order they were made in, each
    // from its first point.
    let mut swept: Vec<ExtrusionPath> = [8.0, 6.0, 4.0]
        .into_iter()
        .map(|y| line((9.0, y), (1.0, y), Role::Ironing, 2))
        .collect();
    for path in &mut swept[1..] {
        path.in_turn = true;
    }
    let text = print(&[layer(0.2, &loops, &[0..1], &swept)], &basic());
    let ends: Vec<(f64, f64)> = feature_lines(&text, "Ironing")
        .iter()
        .filter(|line| word(line, 'E').is_some_and(|e| e > 0.0))
        .filter_map(|line| Some((word(line, 'X')?, word(line, 'Y')?)))
        .collect();
    assert_eq!(ends, [(1.0, 8.0), (1.0, 6.0), (1.0, 4.0)]);
}

#[test]
fn ironing_has_the_fan_speed_set_for_it_and_gives_it_back() {
    let ironing = [line((1.0, 2.0), (9.0, 2.0), Role::Ironing, 2)];
    let fan = |ironing_fan_speed: Option<f64>| PrintSettings {
        ironing_fan_speed,
        close_fan_the_first_x_layers: 1,
        ..basic()
    };
    // None of its own: the fan is left alone, and no marker is left
    // in the file either way.
    let plain = ironed_square(&fan(None), &ironing);
    assert!(!plain.contains(";_FAN_IRONING"));
    let layer_fan = |text: &str| -> Vec<String> {
        layer_text(text, 1)
            .lines()
            .filter(|line| line.starts_with("M106 S"))
            .map(str::to_owned)
            .collect()
    };
    let usual = layer_fan(&plain);
    assert!(usual.len() <= 1, "{usual:?}");

    // 30% while ironing: set once the fill is down, and the layer's
    // own speed again straight after the ironing.
    let text = ironed_square(&fan(Some(30.0)), &ironing);
    assert!(!text.contains(";_FAN_IRONING"));
    let second = layer_text(&text, 1);
    let lines: Vec<&str> = second.lines().collect();
    let at = |wanted: &str| lines.iter().position(|line| *line == wanted);
    let (on, name) = (at("M106 S76").unwrap(), at(";TYPE:Ironing").unwrap());
    assert!(on > at(";TYPE:Top surface").unwrap() && on < name);
    let back = lines[name..]
        .iter()
        .find(|line| line.starts_with("M106 S"))
        .expect("the fan is given back");
    assert_ne!(*back, "M106 S76");
    // The speed it is given back to is the layer's.
    assert_eq!(layer_fan(&text).len(), usual.len() + 2);

    // Not on a layer the fan is kept off for.
    let held = PrintSettings {
        close_fan_the_first_x_layers: 3,
        ..fan(Some(30.0))
    };
    assert!(!ironed_square(&held, &ironing).contains("M106 S76"));
}

#[test]
fn the_ironing_speed_and_fan_are_read_filament_by_filament() {
    let read = |json: &str, filament: usize| {
        PrintSettings::for_filament(&Settings::from_json(json).unwrap(), filament).unwrap()
    };
    // The reference's defaults: 20 mm/s, and no fan speed of its own.
    let unset = read("{}", 0);
    assert_eq!((unset.ironing_speed, unset.ironing_fan_speed), (20.0, None));
    let set = r#"{"ironing_speed": ["30"], "ironing_fan_speed": ["-1", "40"],
                  "filament_ironing_speed": ["nil", "12"]}"#;
    // The first filament sets no speed of its own; the second does.
    assert_eq!(read(set, 0).ironing_speed, 30.0);
    assert_eq!(read(set, 1).ironing_speed, 12.0);
    assert_eq!(read(set, 0).ironing_fan_speed, None);
    assert_eq!(read(set, 1).ironing_fan_speed, Some(40.0));
    // A filament the list has no entry for keeps the process's speed.
    assert_eq!(read(set, 2).ironing_speed, 30.0);
    // A speed that is no number is an error.
    let bad = Settings::from_json(r#"{"ironing_speed": ["slow"]}"#).unwrap();
    assert!(PrintSettings::for_filament(&bad, 0).is_err());
}
