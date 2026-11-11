//! Arcs as the writer lays them: where `G2` and `G3` are written and
//! where they are not, and that everything reckoned along a path still
//! adds up when part of it is an arc.

// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use edge_geom::{ExPolygon, Polygon};

use super::*;

fn flow() -> Flow {
    Flow::new(0.45, 0.2, 0.4).unwrap()
}

/// Points on a circle about (`cx`, `cy`), counter-clockwise from `from`
/// degrees to `to`, `steps` segments.
fn on_circle(cx: f64, cy: f64, r: f64, from: f64, to: f64, steps: usize) -> Vec<Point2> {
    (0..=steps)
        .map(|step| {
            let angle = (from + (to - from) * step as f64 / steps as f64).to_radians();
            Point2::from_mm(cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}

/// A round wall of 128 sides, its first corner half a side past due
/// east, so that no corner is the furthest any way.
fn round_wall(cx: f64, cy: f64, r: f64, depth: usize) -> ExtrusionLoop {
    let half = 360.0 / 256.0;
    let mut points = on_circle(cx, cy, r, half, 360.0 + half, 128);
    points.pop();
    ExtrusionLoop {
        widths: Vec::new(),
        clockwise: false,
        polygon: Polygon { points },
        role: if depth == 0 {
            Role::ExternalPerimeter
        } else {
            Role::Perimeter
        },
        is_hole: false,
        depth,
        flow: flow(),
    }
}

fn square_wall(x: f64, y: f64, side: f64) -> ExtrusionLoop {
    let points = [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
        .map(|(x, y)| Point2::from_mm(x, y));
    ExtrusionLoop {
        polygon: Polygon {
            points: points.to_vec(),
        },
        ..round_wall(0.0, 0.0, 1.0, 0)
    }
}

/// Plain settings with arcs asked for: walls start at the nearest
/// corner and close fully, lifts go straight up, and layers that take no
/// time at all are not slowed down.
fn arcs() -> PrintSettings {
    PrintSettings {
        arc_fitting: true,
        resolution: 0.012,
        seam: Seam::Nearest,
        seam_gap: 0.0,
        lift: Lift::Normal,
        slow_down_for_layer_cooling: false,
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
        z,
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
    }
}

fn print(layers: &[Layer<'_>], settings: &PrintSettings) -> (String, Summary) {
    let mut out = Vec::new();
    let summary = write(&mut out, layers, settings).unwrap();
    (String::from_utf8(out).unwrap(), summary)
}

fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

/// One extruding move read back from the text.
#[derive(Clone, Copy, Debug)]
struct Laid {
    /// 1 for a straight move, 2 and 3 for arcs.
    g: u8,
    from: (f64, f64),
    to: (f64, f64),
    /// Along the move, in mm.
    length: f64,
    fed: f64,
    /// In mm/s.
    speed: f64,
    feature: &'static str,
}

/// Every extruding move of a file, with what is in force as it is made.
/// Every arc is checked on the way: it ends as far from its centre as it
/// starts, to the three places it is written with.
fn laid(text: &str) -> Vec<Laid> {
    let names = [
        "Outer wall",
        "Inner wall",
        "Overhang wall",
        "Sparse infill",
        "Skirt",
        "Brim",
        "Support",
        "Prime tower",
    ];
    let mut out = Vec::new();
    let (mut at, mut speed, mut feature) = (None::<(f64, f64)>, 0.0, "");
    for line in text.lines() {
        if let Some(name) = line.strip_prefix(";TYPE:") {
            feature = names.iter().find(|known| **known == name).unwrap_or(&"");
        }
        let g = match line.split(' ').next() {
            Some("G1") => 1,
            Some("G2") => 2,
            Some("G3") => 3,
            _ => continue,
        };
        speed = word(line, 'F').map_or(speed, |f| f / 60.0);
        let to = match (word(line, 'X'), word(line, 'Y')) {
            (Some(x), Some(y)) => Some((x, y)),
            _ => None,
        };
        if let (Some(from), Some(to), Some(fed)) = (at, to, word(line, 'E'))
            && fed > 0.0
        {
            let length = if g == 1 {
                (to.0 - from.0).hypot(to.1 - from.1)
            } else {
                let (i, j) = (word(line, 'I').unwrap(), word(line, 'J').unwrap());
                let centre = (from.0 + i, from.1 + j);
                let radius = i.hypot(j);
                let end = (to.0 - centre.0).hypot(to.1 - centre.1);
                assert!((radius - end).abs() < 0.003, "off its circle: {line}");
                let angle = |p: (f64, f64)| (p.1 - centre.1).atan2(p.0 - centre.0);
                let turn = angle(to) - angle(from);
                let turn = if g == 3 { turn } else { -turn };
                radius * turn.rem_euclid(TAU)
            };
            out.push(Laid {
                g,
                from,
                to,
                length,
                fed,
                speed,
                feature,
            });
        }
        at = to.or(at);
    }
    out
}

fn arc_count(moves: &[Laid]) -> usize {
    moves.iter().filter(|laid| laid.g != 1).count()
}

#[test]
fn a_round_wall_is_laid_as_arcs_and_adds_up() {
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let layers = [layer(0.2, &loops, &[0..1], &[])];
    let (text, summary) = print(&layers, &arcs());
    let moves = laid(&text);
    // Once round counter-clockwise: the loop's arc, cut in two where it
    // is opened at its seam, and the side that closes it.
    assert!(moves.iter().all(|laid| laid.g != 2), "{text}");
    assert!(
        arc_count(&moves) >= 1 && arc_count(&moves) <= 2,
        "{moves:?}"
    );
    assert!(moves.len() <= 4, "{moves:?}");
    assert_eq!(moves[0].from, moves[moves.len() - 1].to);
    // The arcs are the wall's own circle.
    let arc = text.lines().find(|line| line.starts_with("G3 ")).unwrap();
    let radius = word(arc, 'I').unwrap().hypot(word(arc, 'J').unwrap());
    assert!((radius - 10.0).abs() < 0.01, "{arc}");
    assert!(arc.split(' ').all(|part| {
        // Three places for where it goes and where its centre is.
        !matches!(part.chars().next(), Some('X' | 'Y' | 'I' | 'J'))
            || part
                .split('.')
                .nth(1)
                .is_some_and(|places| places.len() == 3)
    }));

    // As long as the wall is, give or take what the corners cut off.
    let around: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((around - TAU * 10.0).abs() < 0.02, "{around}");
    // Fed by the length laid, arcs by their length along the arc; the
    // file and the summary agree.
    let per_mm = flow().mm3_per_mm() / (PI * 0.875 * 0.875);
    for laid in &moves {
        assert!(
            (laid.fed - laid.length * per_mm).abs() < 2e-4,
            "{laid:?} against {per_mm} a mm"
        );
    }
    let fed: f64 = moves.iter().map(|laid| laid.fed).sum();
    assert!((fed - summary.filament_mm).abs() < 1e-4);
    assert!((summary.filament_mm - around * per_mm).abs() < 1e-3);

    // Without arcs asked for it is 128 straight moves, as it always was.
    let straight = PrintSettings {
        arc_fitting: false,
        ..arcs()
    };
    let (text, plain) = print(&layers, &straight);
    assert!(!text.contains("\nG2 ") && !text.contains("\nG3 "));
    assert_eq!(laid(&text).len(), 128);
    // The arcs lay as much, to within what the corners cut off and what
    // the fitted circle is off the wall's own.
    assert!((summary.filament_mm - plain.filament_mm).abs() < plain.filament_mm * 0.0005);
}

#[test]
fn a_seam_gap_shortens_the_last_arc() {
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let layers = [layer(0.2, &loops, &[0..1], &[])];
    let gap = PrintSettings {
        seam_gap: 0.5,
        ..arcs()
    };
    let (closed, _) = print(&layers, &arcs());
    let (text, _) = print(&layers, &gap);
    let (whole, short) = (laid(&closed), laid(&text));
    let length = |moves: &[Laid]| moves.iter().map(|laid| laid.length).sum::<f64>();
    assert!((length(&whole) - length(&short) - 0.5).abs() < 0.01);
    // It stops half a millimetre short of where it started, on the wall.
    let (start, end) = (short[0].from, short[short.len() - 1].to);
    let apart = (end.0 - start.0).hypot(end.1 - start.1);
    assert!((apart - 0.5).abs() < 0.01, "{apart}");
    assert!(((end.0 - 50.0).hypot(end.1 - 50.0) - 10.0).abs() < 0.02);
    assert!(arc_count(&short) >= 1);

    // A gap that takes the whole of the loop's last stretch and some of
    // the arc before it ends the arc early, on its circle.
    let wide = PrintSettings {
        seam_gap: 3.0,
        ..arcs()
    };
    let shorter = laid(&print(&layers, &wide).0);
    assert!((length(&whole) - length(&shorter) - 3.0).abs() < 0.01);
    let last = shorter[shorter.len() - 1];
    assert_eq!(last.g, 3, "{shorter:?}");
    assert!(((last.to.0 - 50.0).hypot(last.to.1 - 50.0) - 10.0).abs() < 0.005);
}

#[test]
fn a_hole_and_a_wall_laid_clockwise_turn_the_other_way() {
    // A hole's points run clockwise and it is laid counter-clockwise
    // like any wall; asked to lay walls clockwise, both go as `G2`.
    let mut hole = round_wall(50.0, 50.0, 6.0, 0);
    hole.polygon.points.reverse();
    hole.is_hole = true;
    let loops = [hole, round_wall(50.0, 50.0, 10.0, 0)];
    let layers = [layer(0.2, &loops, &[0..2], &[])];
    let (text, _) = print(&layers, &arcs());
    let moves = laid(&text);
    assert!(arc_count(&moves) >= 2);
    assert!(moves.iter().all(|laid| laid.g != 2), "{moves:?}");

    let clockwise = PrintSettings {
        wall_direction: WallDirection::Clockwise,
        ..arcs()
    };
    let (text, _) = print(&layers, &clockwise);
    let moves = laid(&text);
    assert!(arc_count(&moves) >= 2);
    assert!(moves.iter().all(|laid| laid.g != 3), "{moves:?}");
    let around: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((around - TAU * 16.0).abs() < 0.03, "{around}");
}

#[test]
fn a_curved_line_keeps_its_arc_whichever_end_it_is_laid_from() {
    // A quarter turn of fill, and a wall to leave the nozzle at one end
    // of it or the other.
    let quarter = ExtrusionPath {
        points: on_circle(50.0, 50.0, 8.0, 0.0, 90.0, 48),
        role: Role::SparseInfill,
        flow: flow(),
        in_turn: false,
        part: 0,
    };
    let paths = [quarter];
    for (near, turn) in [((58.0, 49.0), 3), ((49.0, 58.0), 2)] {
        let loops = [square_wall(near.0, near.1, 0.5)];
        let layers = [layer(0.2, &loops, &[0..1], &paths)];
        let (text, _) = print(&layers, &arcs());
        let fill: Vec<Laid> = laid(&text)
            .into_iter()
            .filter(|laid| laid.feature == "Sparse infill")
            .collect();
        assert_eq!(fill.len(), 1, "{fill:?}");
        assert_eq!(fill[0].g, turn);
        assert!((fill[0].length - 8.0 * PI / 2.0).abs() < 0.01);
        let (from, to) = if turn == 3 {
            ((58.0, 50.0), (50.0, 58.0))
        } else {
            ((50.0, 58.0), (58.0, 50.0))
        };
        assert_eq!((fill[0].from, fill[0].to), (from, to));
    }
    // A line of two points is a straight move however arcs are asked for.
    let straight = [ExtrusionPath {
        points: vec![Point2::from_mm(40.0, 40.0), Point2::from_mm(60.0, 40.0)],
        ..paths[0].clone()
    }];
    let (text, _) = print(&[layer(0.2, &[], &[], &straight)], &arcs());
    assert_eq!(arc_count(&laid(&text)), 0);
}

#[test]
fn sparse_infill_and_supports_are_held_less_closely() {
    // A quarter turn of 12 mm radius in twelve sides, each 26 µm from
    // the circle at its middle: within what sparse infill (40 µm) and
    // supports (37.5 µm) are allowed, not within the 12 µm of the rest.
    let coarse = on_circle(50.0, 50.0, 12.0, 0.0, 90.0, 12);
    let path = |role: Role| ExtrusionPath {
        points: coarse.clone(),
        role,
        flow: flow(),
        in_turn: false,
        part: 0,
    };
    let laid_as = |role: Role, as_support: bool| {
        let paths = [path(role)];
        let mut one = layer(0.2, &[], &[], &[]);
        if as_support {
            one.support = &paths;
        } else {
            one.paths = &paths;
        }
        let moves = laid(&print(&[one], &arcs()).0);
        (arc_count(&moves), moves.len())
    };
    assert_eq!(laid_as(Role::SparseInfill, false), (1, 1));
    assert_eq!(laid_as(Role::Support, true), (1, 1));
    assert_eq!(laid_as(Role::SupportInterface, true), (1, 1));
    assert_eq!(laid_as(Role::SolidInfill, false), (0, 12));
    assert_eq!(laid_as(Role::TopSolidInfill, false), (0, 12));
}

#[test]
fn skirt_brim_and_prime_tower_get_no_arcs() {
    let around = [
        ExtrusionLoop {
            role: Role::Skirt,
            ..round_wall(50.0, 50.0, 14.0, 0)
        },
        ExtrusionLoop {
            role: Role::Brim,
            ..round_wall(50.0, 50.0, 11.0, 0)
        },
    ];
    let tower = [ExtrusionPath {
        points: on_circle(90.0, 50.0, 5.0, 0.0, 180.0, 64),
        role: Role::PrimeTower,
        flow: flow(),
        in_turn: false,
        part: 0,
    }];
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let mut one = layer(0.2, &loops, &[0..1], &[]);
    one.around = &around;
    one.tower = &tower;
    let (text, _) = print(&[one], &arcs());
    let moves = laid(&text);
    for feature in ["Skirt", "Brim", "Prime tower"] {
        let own: Vec<&Laid> = moves
            .iter()
            .filter(|laid| laid.feature == feature)
            .collect();
        assert!(own.len() >= 64, "{feature}: {}", own.len());
        assert!(own.iter().all(|laid| laid.g == 1), "{feature}");
    }
    assert!(
        moves
            .iter()
            .any(|laid| laid.feature == "Outer wall" && laid.g == 3)
    );
}

#[test]
fn a_spiral_lift_is_one_arc() {
    let far = [square_wall(0.0, 0.0, 10.0), square_wall(50.0, 0.0, 10.0)];
    let settings = PrintSettings {
        lift: Lift::Spiral,
        bed: Some([-20.0, -20.0, 100.0, 100.0]),
        ..arcs()
    };
    let layers = [layer(0.2, &far, &[0..1, 1..2], &[])];
    let (text, with_arc) = print(&layers, &settings);
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|line| *line == "G17").unwrap();
    // A whole turn to the left of the way the travel heads, east: its
    // centre is 1.217 mm north of the nozzle, and it ends 0.4 mm up.
    let turn = lines[at + 1];
    assert!(turn.starts_with("G3 Z0.600 I0.000 J1.217 P1 F"), "{turn}");
    assert!(!turn.contains('X') && !turn.contains('E'));
    // No faster than the Z axis can climb 0.4 mm over the turn.
    let z_speed = settings.z_speed;
    let around = TAU * 1.217;
    let most = z_speed * around.hypot(0.4) / 0.4;
    let expected = settings.travel_speed.min(most) * 60.0;
    assert!((word(turn, 'F').unwrap() - expected).abs() <= 1.0, "{turn}");
    assert!(lines[at + 2].starts_with("G1 X50.000 Y0.000 Z0.600"));
    assert_eq!(lines.iter().filter(|line| **line == "G17").count(), 1);

    // Straight moves do the same turn where arcs are not asked for, and
    // go as far.
    let segmented = PrintSettings {
        arc_fitting: false,
        ..settings.clone()
    };
    let (text, without) = print(&layers, &segmented);
    assert!(!text.contains("G17") && !text.contains("G3 "));
    assert!((with_arc.travel_mm - without.travel_mm).abs() < 0.05);
    // Off the edge of the bed it goes straight up, arcs or no.
    let cramped = PrintSettings {
        bed: Some([-1.0, -1.0, 100.0, 100.0]),
        ..settings
    };
    let (text, _) = print(&layers, &cramped);
    assert!(!text.contains("G17") && text.contains("G1 Z0.600 F"));
}

#[test]
fn an_arc_that_bulges_off_the_bed_stops_the_file() {
    // The wall's corners are all within 59.997 mm of the front edge;
    // the arc fitted to them reaches a micron or two short of 60.
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let layers = [layer(0.2, &loops, &[0..1], &[])];
    let furthest = (loops[0].polygon.points.iter())
        .map(|p| p.y)
        .max()
        .map(edge_geom::unscaled)
        .unwrap();
    assert!(furthest < 59.9975);
    let tight = PrintSettings {
        bed: Some([0.0, 0.0, 100.0, 59.9975]),
        ..arcs()
    };
    let mut out = Vec::new();
    let error = write(&mut out, &layers, &tight).unwrap_err();
    assert!(out.is_empty(), "nothing is written of a file that fails");
    match &error {
        GcodeError::OffBed { reach, bed } => {
            assert!(reach[3] > 59.9975 && reach[3] < 60.001, "{reach:?}");
            assert_eq!(bed[3], 59.9975);
        }
        other => panic!("{other}"),
    }
    assert!(error.to_string().contains("off the bed"));
    // The same wall as straight moves stays on it, and so does the arc
    // with room for its bulge.
    let straight = PrintSettings {
        arc_fitting: false,
        ..tight.clone()
    };
    assert!(write(&mut Vec::new(), &layers, &straight).is_ok());
    let roomy = PrintSettings {
        bed: Some([0.0, 0.0, 100.0, 60.002]),
        ..tight
    };
    assert!(write(&mut Vec::new(), &layers, &roomy).is_ok());
}

/// Settings that measure a wall against the layer beneath: slowed where
/// it overhangs, a bridge where it hangs clear.
fn measured() -> PrintSettings {
    PrintSettings {
        outer_wall_speed: 200.0,
        bridge_speed: 25.0,
        enable_overhang_speed: true,
        detect_overhang_wall: true,
        overhang_speeds: [
            FloatOrPercent::Value(0.0),
            FloatOrPercent::Value(50.0),
            FloatOrPercent::Value(30.0),
            FloatOrPercent::Value(10.0),
        ],
        overhang_fan: None,
        ..arcs()
    }
}

fn plate(x0: f64, y0: f64, x1: f64, y1: f64) -> [ExPolygon; 1] {
    [ExPolygon {
        contour: Polygon {
            points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        },
        holes: Vec::new(),
    }]
}

#[test]
fn a_wall_keeps_its_arcs_where_it_is_laid_one_way_throughout() {
    // A round wall with a plate under all of it: measured, found to be
    // supported all the way round, and laid as the arcs it is.
    let under = plate(30.0, 30.0, 70.0, 70.0);
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let mut second = layer(0.4, &loops, &[0..1], &[]);
    second.below = &under;
    let layers = [layer(0.2, &[], &[], &[]), second];
    let (text, summary) = print(&layers, &measured());
    let moves = laid(&text);
    assert!(arc_count(&moves) >= 1 && moves.len() <= 4, "{moves:?}");
    assert!(moves.iter().all(|laid| laid.speed == 200.0));
    assert!(moves.iter().all(|laid| laid.feature == "Outer wall"));
    let around: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((around - TAU * 10.0).abs() < 0.02, "{around}");
    let fed: f64 = moves.iter().map(|laid| laid.fed).sum();
    assert!((fed - summary.filament_mm).abs() < 1e-4);
}

#[test]
fn a_wall_that_slows_on_its_way_is_laid_straight_and_a_bridge_keeps_its_arc() {
    // The same wall over a plate that reaches under its left half only.
    // Over the plate its speed changes where it comes off the edge, so
    // that whole stretch is straight moves, as in the reference, which
    // gives up a path's arcs when its speed is not one throughout. Over
    // air it is a bridge at the bridge's speed all the way: an arc.
    let under = plate(30.0, 30.0, 50.0, 70.0);
    let loops = [round_wall(50.0, 50.0, 10.0, 0)];
    let mut second = layer(0.4, &loops, &[0..1], &[]);
    second.below = &under;
    let layers = [layer(0.2, &[], &[], &[]), second];
    let settings = measured();
    let (text, summary) = print(&layers, &settings);
    let moves = laid(&text);
    let wall: Vec<&Laid> = moves
        .iter()
        .filter(|laid| laid.feature == "Outer wall")
        .collect();
    let over_air: Vec<&Laid> = moves
        .iter()
        .filter(|laid| laid.feature == "Overhang wall")
        .collect();
    assert!(wall.len() > 40 && wall.iter().all(|laid| laid.g == 1));
    assert!(wall.iter().any(|laid| laid.speed == 200.0));
    assert!(
        wall.iter()
            .any(|laid| laid.speed < 199.0 && laid.speed >= 10.0)
    );
    assert!(
        over_air
            .iter()
            .any(|laid| laid.g != 1 && laid.speed == 25.0 && laid.length > 10.0),
        "{over_air:?}"
    );
    assert!(over_air.iter().all(|laid| laid.speed <= 25.0));
    // No arc is laid at a speed that is not its whole stretch's.
    assert!(
        moves
            .iter()
            .filter(|laid| laid.g != 1)
            .all(|laid| laid.speed == 25.0)
    );
    // Once round, and the file agrees with the summary.
    let around: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((around - TAU * 10.0).abs() < 0.05, "{around}");
    let fed: f64 = moves.iter().map(|laid| laid.fed).sum();
    assert!((fed - summary.filament_mm).abs() < 1e-3);

    // Without the arcs it is the same wall, as long and as fast.
    let straight = PrintSettings {
        arc_fitting: false,
        ..settings
    };
    let (text, plain) = print(&layers, &straight);
    let plain_moves = laid(&text);
    assert_eq!(arc_count(&plain_moves), 0);
    let plain_around: f64 = plain_moves.iter().map(|laid| laid.length).sum();
    assert!((around - plain_around).abs() < 0.05);
    assert!((summary.filament_mm - plain.filament_mm).abs() < 0.005 * plain.filament_mm);
    assert!((summary.seconds_at_speeds - plain.seconds_at_speeds).abs() < 0.05);
}

#[test]
fn a_layer_slowed_to_cool_keeps_its_arcs() {
    // Two layers of a small round wall, which take no time at all: laid
    // out a second time, slower, with the arcs fitted the first time.
    let loops = [round_wall(50.0, 50.0, 3.0, 0)];
    let layers = [
        layer(0.2, &loops, &[0..1], &[]),
        layer(0.4, &loops, &[0..1], &[]),
    ];
    let settings = PrintSettings {
        slow_down_for_layer_cooling: true,
        slow_down_layer_time: 20.0,
        slow_down_min_speed: 10.0,
        ..arcs()
    };
    let (text, summary) = print(&layers, &settings);
    assert_eq!(summary.slowed_layers, 2);
    let moves = laid(&text);
    assert!(arc_count(&moves) >= 2);
    assert!(moves.iter().all(|laid| laid.speed <= 10.0 + 1e-9));
    // The same arcs as when it is not slowed.
    let unhurried = PrintSettings {
        slow_down_for_layer_cooling: false,
        ..settings
    };
    let (text, _) = print(&layers, &unhurried);
    let fast = laid(&text);
    assert_eq!(fast.len(), moves.len());
    for (a, b) in fast.iter().zip(&moves) {
        assert_eq!((a.g, a.from, a.to), (b.g, b.from, b.to));
    }
}

#[test]
fn a_wipe_follows_the_points_of_the_arc() {
    // After a round wall the nozzle is wiped on round it: straight
    // moves through the points the arc was fitted to, not off across
    // the circle.
    let loops = [
        round_wall(50.0, 50.0, 10.0, 0),
        square_wall(100.0, 50.0, 5.0),
    ];
    let layers = [layer(0.2, &loops, &[0..1, 1..2], &[])];
    let settings = PrintSettings {
        wipe: true,
        wipe_distance: 2.0,
        retraction_length: 0.8,
        ..arcs()
    };
    let (text, _) = print(&layers, &settings);
    let wipe: Vec<&str> = text
        .lines()
        .skip_while(|line| *line != ";WIPE_START")
        .take_while(|line| *line != ";WIPE_END")
        .filter(|line| line.starts_with("G1 X"))
        .collect();
    assert!(wipe.len() >= 2, "{wipe:?}");
    for line in &wipe {
        let (x, y) = (word(line, 'X').unwrap(), word(line, 'Y').unwrap());
        let off = ((x - 50.0).hypot(y - 50.0) - 10.0).abs();
        assert!(off < 0.02, "{line} is {off} mm off the wall");
    }
}

#[test]
fn settings_read_arc_fitting_and_never_with_spiral_mode() {
    let read = |json: &str| {
        PrintSettings::from_settings(&edge_config::Settings::from_json(json).unwrap()).unwrap()
    };
    assert!(!read("{}").arc_fitting);
    let on = read(r#"{"enable_arc_fitting": "1"}"#);
    assert!(on.arc_fitting);
    // It is done, so nothing is said of it.
    assert!(
        on.unsupported
            .iter()
            .all(|note| !note.contains("enable_arc_fitting")),
        "{:?}",
        on.unsupported
    );
    assert!(!read(r#"{"enable_arc_fitting": "1", "spiral_mode": "1"}"#).arc_fitting);
}
