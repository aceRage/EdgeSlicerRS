//! Travel that goes round the inside of the walls (`reduce_crossing_wall`).

use edge_geom::Polygon;

use super::*;

fn flow() -> Flow {
    Flow::new(0.45, 0.2, 0.4).unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Polygon {
    Polygon {
        points: [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec(),
    }
}

/// A 40 mm square with a 20 mm square hole in the middle of it.
fn frame() -> ExPolygon {
    let mut hole = rect(10.0, 10.0, 30.0, 30.0);
    hole.points.reverse();
    ExPolygon {
        contour: rect(0.0, 0.0, 40.0, 40.0),
        holes: vec![hole],
    }
}

fn line(a: (f64, f64), b: (f64, f64), role: Role) -> ExtrusionPath {
    ExtrusionPath {
        points: vec![Point2::from_mm(a.0, a.1), Point2::from_mm(b.0, b.1)],
        role,
        flow: flow(),
        // Laid in the order and the direction given.
        in_turn: true,
        part: 0,
    }
}

fn skirt(x0: f64, y0: f64, x1: f64, y1: f64, role: Role) -> ExtrusionLoop {
    ExtrusionLoop {
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
        polygon: rect(x0, y0, x1, y1),
        role,
        is_hole: false,
        depth: 0,
        flow: flow(),
    }
}

/// The reference's defaults, less what would get in the way of reading a
/// small test, with travel sent round the walls.
fn routed() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        seam_gap: 0.0,
        lift: Lift::Normal,
        slow_down_for_layer_cooling: false,
        wipe: false,
        reduce_crossing_wall: true,
        ..PrintSettings::default()
    }
}

fn layer<'a>(z: f64, paths: &'a [ExtrusionPath], shelter: &'a [ExPolygon]) -> Layer<'a> {
    Layer {
        region: None,
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        tower: &[],
        around: &[],
        support: &[],
        loops: &[],
        islands: &[],
        paths,
        shelter,
        below: &[],
    }
}

fn slices(outline: &[ExPolygon]) -> Slices<'_> {
    Slices {
        outline,
        tops: &[],
        spacing: 0.4,
        outer_width: 0.42,
    }
}

fn print(layers: &[Layer<'_>], by: &[Slices<'_>], settings: &PrintSettings) -> (String, Summary) {
    let mut out = Vec::new();
    let summary = write_routed(&mut out, layers, by, std::slice::from_ref(settings)).unwrap();
    (String::from_utf8(out).unwrap(), summary)
}

fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

/// Every travel of the print after its first: the points gone through,
/// the first being where the nozzle was, each with the height said on
/// the move to it, if any.
fn travels(text: &str) -> Vec<Vec<(f64, f64, Option<f64>)>> {
    let mut at = None;
    let mut found: Vec<Vec<(f64, f64, Option<f64>)>> = Vec::new();
    let mut travelling = false;
    for line in text.lines().filter(|line| line.starts_with("G1 ")) {
        let (Some(x), Some(y)) = (word(line, 'X'), word(line, 'Y')) else {
            continue;
        };
        if word(line, 'E').is_some() {
            travelling = false;
        } else if let Some(from) = at {
            if !travelling {
                found.push(vec![from]);
                travelling = true;
            }
            let run = found.last_mut().unwrap();
            run.push((x, y, word(line, 'Z')));
        }
        at = Some((x, y, None));
    }
    found
}

/// Whether the move from `a` to `b` passes over the frame's hole.
fn over_the_hole(a: (f64, f64, Option<f64>), b: (f64, f64, Option<f64>)) -> bool {
    (0..=100).any(|step| {
        let t = f64::from(step) / 100.0;
        let (x, y) = (a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
        x > 10.0 && x < 30.0 && y > 10.0 && y < 30.0
    })
}

/// A line of fill down the left side of the frame and one down the right.
fn two_sides() -> [ExtrusionPath; 2] {
    [
        line((3.0, 18.0), (7.0, 18.0), Role::SparseInfill),
        line((33.0, 18.0), (37.0, 18.0), Role::SparseInfill),
    ]
}

#[test]
fn a_travel_across_a_hole_goes_round_it() {
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &[])];
    let by = [slices(&outline)];

    let straight = PrintSettings {
        reduce_crossing_wall: false,
        ..routed()
    };
    let (text, direct) = print(&layers, &by, &straight);
    let moves = travels(&text);
    assert_eq!(moves.len(), 1);
    assert_eq!(moves[0].len(), 2);
    assert!(over_the_hole(moves[0][0], moves[0][1]));

    let (text, summary) = print(&layers, &by, &routed());
    let moves = travels(&text);
    assert_eq!(moves.len(), 1);
    let way = &moves[0];
    assert!(way.len() > 3, "{way:?}");
    assert_eq!((way[0].0, way[0].1), (7.0, 18.0));
    assert_eq!((way[way.len() - 1].0, way[way.len() - 1].1), (33.0, 18.0));
    assert!(way.windows(2).all(|pair| !over_the_hole(pair[0], pair[1])));
    // Under the hole, 0.6 mm clear of it: 26 mm straight, and some 40 round.
    let longer = summary.travel_mm - direct.travel_mm;
    assert!(longer > 10.0 && longer < 20.0, "{longer}");
    // The lift is straight up before the first of it, and nothing after
    // says a height: the whole way is gone at the one.
    assert!(way[1..].iter().all(|point| point.2.is_none()), "{way:?}");
    assert_eq!(summary.retractions, direct.retractions);
    // The file's own count of travel agrees with what was written.
    let written: f64 = way
        .windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
        .sum();
    assert!((written - 26.0 - longer).abs() < 0.01, "{written}");
}

#[test]
fn a_ramped_lift_is_made_over_the_first_stretch_of_the_way() {
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &[])];
    let settings = PrintSettings {
        lift: Lift::Slope,
        ..routed()
    };
    let (text, _) = print(&layers, &[slices(&outline)], &settings);
    let way = &travels(&text)[0];
    // Up to the hop's height on the first stretch, and no height after.
    let heights: Vec<f64> = way.iter().filter_map(|point| point.2).collect();
    assert!(!heights.is_empty() && heights.iter().all(|z| (z - 0.6).abs() < 1e-9));
    let first_flat = way
        .iter()
        .skip(1)
        .position(|point| point.2.is_none())
        .unwrap();
    assert!(way[1 + first_flat..].iter().all(|point| point.2.is_none()));
    assert!(way.windows(2).all(|pair| !over_the_hole(pair[0], pair[1])));
}

#[test]
fn a_travel_that_stays_inside_the_part_may_go_unretracted() {
    // With somewhere to shelter (`reduce_infill_retraction`) the straight
    // move over the hole retracts, and the way round it, which never
    // leaves the part, does not.
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &outline)];
    let by = [slices(&outline)];
    let sheltered = PrintSettings {
        reduce_infill_retraction: true,
        ..routed()
    };
    let straight = PrintSettings {
        reduce_crossing_wall: false,
        ..sheltered.clone()
    };
    // The retraction that ends the print is the other one.
    assert_eq!(print(&layers, &by, &straight).1.retractions, 2);
    let (text, summary) = print(&layers, &by, &sheltered);
    assert_eq!(summary.retractions, 1);
    assert!(travels(&text)[0].len() > 3);
    // Without the shelter the way round retracts like any long move.
    assert_eq!(print(&layers, &by, &routed()).1.retractions, 2);

    // A move is as long as its way round: the hop from one side of a
    // thin slot to the other is under `retraction_minimum_travel`
    // straight and well over it round the slot's end.
    let mut slot = rect(19.5, 5.0, 20.5, 35.0);
    slot.points.reverse();
    let slotted = [ExPolygon {
        contour: rect(0.0, 0.0, 40.0, 40.0),
        holes: vec![slot],
    }];
    let fill = [
        line((15.0, 20.0), (19.0, 20.0), Role::SparseInfill),
        line((21.0, 20.0), (25.0, 20.0), Role::SparseInfill),
    ];
    let layers = [layer(0.2, &fill, &[])];
    let by = [slices(&slotted)];
    let straight = PrintSettings {
        reduce_crossing_wall: false,
        ..routed()
    };
    assert_eq!(print(&layers, &by, &straight).1.retractions, 1);
    let (_, summary) = print(&layers, &by, &routed());
    assert_eq!(summary.retractions, 2);
    assert!(summary.travel_mm > 30.0);
}

#[test]
fn a_way_round_over_the_limit_is_not_taken() {
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &[])];
    let by = [slices(&outline)];
    let straight = PrintSettings {
        reduce_crossing_wall: false,
        ..routed()
    };
    let (direct, _) = print(&layers, &by, &straight);
    let (round, _) = print(&layers, &by, &routed());
    assert_ne!(round, direct);
    // The way round is some 14 mm longer than the 26 mm straight.
    for (limit, taken) in [
        (Detour::Length(5.0), false),
        (Detour::Length(25.0), true),
        (Detour::Percent(20.0), false),
        (Detour::Percent(90.0), true),
    ] {
        let limited = PrintSettings {
            max_travel_detour: Some(limit),
            ..routed()
        };
        let (text, _) = print(&layers, &by, &limited);
        assert_eq!(text == round, taken, "{limit:?}");
        assert_eq!(text == direct, !taken, "{limit:?}");
    }
}

#[test]
fn travel_goes_straight_without_the_setting_or_without_slices() {
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &[])];
    let straight = PrintSettings {
        reduce_crossing_wall: false,
        ..routed()
    };
    let mut plain = Vec::new();
    write(&mut plain, &layers, &straight).unwrap();
    let plain = String::from_utf8(plain).unwrap();
    // The setting off, whatever is given; and on, with nothing to go by.
    assert_eq!(print(&layers, &[slices(&outline)], &straight).0, plain);
    assert_eq!(print(&layers, &[], &routed()).0, plain);
}

#[test]
fn the_first_move_and_the_one_from_a_skirt_go_straight() {
    // Fill on the right of the frame only, and a loop of skirt that
    // lies wholly to the left of it, with the hole between the two.
    let fill = [line((33.0, 18.0), (37.0, 18.0), Role::SparseInfill)];
    let around = [skirt(-5.0, -5.0, -3.0, 45.0, Role::Skirt)];
    let outline = [frame()];
    let with_skirt = |z: f64| Layer {
        around: &around,
        ..layer(z, &fill, &[])
    };
    let layers = [with_skirt(0.2), with_skirt(0.4)];
    let by = [slices(&outline), slices(&outline)];
    let (text, _) = print(&layers, &by, &routed());
    let moves = travels(&text);
    // The first layer: from the skirt straight to the fill, as the
    // reference allows on the first layer alone, though that is over
    // the hole.
    let from_skirt = &moves[0];
    assert_eq!(from_skirt.len(), 2, "{from_skirt:?}");
    assert!(over_the_hole(from_skirt[0], from_skirt[1]));
    // The next: to the skirt is a move outside the part, round the hole
    // in the hole's own wall; and from the skirt it is routed like any
    // move in the part.
    let to_skirt = &moves[1];
    assert!(to_skirt.len() > 2, "{to_skirt:?}");
    assert!(
        to_skirt
            .windows(2)
            .all(|pair| !over_the_hole(pair[0], pair[1]))
    );
    let from_skirt = &moves[2];
    assert!(from_skirt.len() > 2, "{from_skirt:?}");
    assert!(
        from_skirt
            .windows(2)
            .all(|pair| !over_the_hole(pair[0], pair[1]))
    );

    // From a brim the move goes straight on whatever layer.
    let around = [skirt(-5.0, -5.0, -3.0, 45.0, Role::Brim)];
    let with_brim = |z: f64| Layer {
        around: &around,
        ..layer(z, &fill, &[])
    };
    let layers = [with_brim(0.2), with_brim(0.4)];
    let (text, _) = print(&layers, &by, &routed());
    let moves = travels(&text);
    assert_eq!(moves[2].len(), 2, "{:?}", moves[2]);
    assert!(over_the_hole(moves[2][0], moves[2][1]));
}

#[test]
fn moves_between_supports_are_routed_by_the_part() {
    // Two support lines, one on each side of a solid block: the move
    // between them is taken along the inside of the block, as the
    // reference takes it, where a move in the part that crossed no wall
    // would go straight.
    let block = [ExPolygon {
        contour: rect(10.0, 0.0, 30.0, 40.0),
        holes: Vec::new(),
    }];
    let support = [
        line((2.0, 2.0), (2.0, 6.0), Role::Support),
        line((38.0, 2.0), (38.0, 6.0), Role::Support),
    ];
    let layers = [Layer {
        support: &support,
        ..layer(0.2, &[], &[])
    }];
    let (text, _) = print(&layers, &[slices(&block)], &routed());
    let moves = travels(&text);
    assert_eq!(moves.len(), 1);
    assert!(moves[0].len() > 2, "{:?}", moves[0]);
    // In at 0.6 mm from the block's edge and out at as much from the other.
    assert!((moves[0][1].0 - 10.6).abs() < 0.01, "{:?}", moves[0]);
}

#[test]
fn routed_travel_is_the_same_on_any_number_of_threads_and_when_slowed() {
    // More layers than are made ready at once, each with the move over
    // the hole and back.
    let outline = [frame()];
    let fill: Vec<ExtrusionPath> = two_sides()
        .into_iter()
        .chain([line((3.0, 22.0), (7.0, 22.0), Role::SparseInfill)])
        .collect();
    let layers: Vec<Layer<'_>> = (1..=FITTED_AHEAD + 6)
        .map(|index| layer(0.2 * index as f64, &fill, &[]))
        .collect();
    let by: Vec<Slices<'_>> = layers.iter().map(|_| slices(&outline)).collect();
    let on = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| print(&layers, &by, &routed()).0)
    };
    let one = on(1);
    assert_eq!(on(2), one);
    assert_eq!(on(7), one);
    let ways = travels(&one);
    assert!(ways.iter().filter(|way| way.len() > 3).count() >= 2 * layers.len() - 1);

    // A layer laid out a second time, slower, travels the same way.
    let slowed = PrintSettings {
        slow_down_for_layer_cooling: true,
        ..routed()
    };
    let (text, summary) = print(&layers, &by, &slowed);
    assert_eq!(summary.slowed_layers, layers.len());
    let points = |ways: Vec<Vec<(f64, f64, Option<f64>)>>| -> Vec<Vec<(f64, f64)>> {
        ways.into_iter()
            .map(|way| way.into_iter().map(|point| (point.0, point.1)).collect())
            .collect()
    };
    assert_eq!(points(travels(&text)), points(ways));
}

#[test]
fn a_routed_travel_stays_on_the_bed() {
    // The part is on the bed, so every way round it is; a bed the part
    // itself were off would stop the file being written at all. Here
    // the bed is cut down to where the way under the hole would leave
    // it, and the travel goes straight instead.
    let (fill, outline) = (two_sides(), [frame()]);
    let layers = [layer(0.2, &fill, &[])];
    let by = [slices(&outline)];
    let roomy = PrintSettings {
        bed: Some([0.0, 0.0, 40.0, 40.0]),
        ..routed()
    };
    let (text, _) = print(&layers, &by, &roomy);
    assert!(travels(&text)[0].len() > 3);
    let cramped = PrintSettings {
        bed: Some([0.0, 9.5, 40.0, 40.0]),
        ..routed()
    };
    let (text, _) = print(&layers, &by, &cramped);
    let way = &travels(&text)[0];
    assert!(way.iter().all(|point| point.1 >= 9.5), "{way:?}");
}

#[test]
fn the_settings_are_read() {
    let read = |json: &str| {
        PrintSettings::from_settings(&edge_config::Settings::from_json(json).unwrap()).unwrap()
    };
    let unset = read("{}");
    assert!(!unset.reduce_crossing_wall);
    assert_eq!(unset.max_travel_detour, None);
    let on = read(r#"{"reduce_crossing_wall": "1", "max_travel_detour_distance": "0"}"#);
    assert!(on.reduce_crossing_wall);
    assert_eq!(on.max_travel_detour, None);
    let length = read(r#"{"reduce_crossing_wall": "1", "max_travel_detour_distance": "12.5"}"#);
    assert_eq!(length.max_travel_detour, Some(Detour::Length(12.5)));
    let share = read(r#"{"max_travel_detour_distance": "50%"}"#);
    assert_eq!(share.max_travel_detour, Some(Detour::Percent(50.0)));
    assert!(
        read(r#"{"max_travel_detour_distance": "0%"}"#)
            .max_travel_detour
            .is_none()
    );
}

#[test]
fn a_way_round_is_sheltered_as_a_straight_move_is() {
    // The areas sorted into cells answer as looking at every edge
    // does: a frame, an island in its hole, and a triangle apart, asked
    // about moves all over them, on their edges and through their
    // corners.
    let island = ExPolygon {
        contour: rect(14.0, 14.0, 26.0, 26.0),
        holes: Vec::new(),
    };
    let triangle = ExPolygon {
        contour: Polygon {
            points: [(50.0, 0.0), (70.0, 10.5), (55.5, 30.0)]
                .map(|(x, y)| Point2::from_mm(x, y))
                .to_vec(),
        },
        holes: Vec::new(),
    };
    let areas = [frame(), island, triangle];
    let sorted = edge_avoid::Shelter::new(&areas);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = |span: f64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        // Half of them on a 0.5 mm lattice, to land on edges and corners.
        let value = (seed >> 11) as f64 / (1u64 << 53) as f64 * span - 5.0;
        if seed & 1 == 0 {
            (value * 2.0).round() / 2.0
        } else {
            value
        }
    };
    let (mut inside_count, mut outside_count) = (0, 0);
    for _ in 0..20_000 {
        let a = Point2::from_mm(next(80.0), next(45.0));
        let b = Point2::from_mm(next(80.0), next(45.0));
        let wanted = inside::stays_inside(&areas, a, b);
        assert_eq!(sorted.holds(a, b), wanted, "{a:?} {b:?}");
        if wanted {
            inside_count += 1;
        } else {
            outside_count += 1;
        }
    }
    assert!(inside_count > 500 && outside_count > 500);
    // Short moves, most of which stay inside.
    for _ in 0..20_000 {
        let (x, y) = (next(80.0), next(45.0));
        let a = Point2::from_mm(x, y);
        let b = Point2::from_mm(x + next(12.0) / 4.0, y + next(12.0) / 4.0);
        assert_eq!(
            sorted.holds(a, b),
            inside::stays_inside(&areas, a, b),
            "{a:?} {b:?}"
        );
    }
    assert!(
        !edge_avoid::Shelter::new(&[]).holds(Point2::from_mm(0.0, 0.0), Point2::from_mm(1.0, 1.0))
    );
}
