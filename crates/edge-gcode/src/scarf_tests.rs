//! Scarf seams: how a loop is cut into its way up, its flat part and its
//! way down, which loops get one, and what the writer makes of them.

// A list holding one island reads as `&[0..1]`, which is what is meant.
#![allow(clippy::single_range_in_vec_init)]

use edge_config::{ConfigError, Settings};
use edge_geom::{ExPolygon, Polygon, UNITS_PER_MM};

use super::*;
use crate::scarf::{Run, Sloped, is_smooth, sloped};

fn flow() -> Flow {
    Flow::new(0.45, 0.2, 0.4).unwrap()
}

fn mm(x: f64, y: f64) -> Point2 {
    Point2::from_mm(x, y)
}

/// Length of a path in mm.
fn long(points: &[Point2]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .sum()
}

/// A square of `side` mm with its corner at (`x`, `y`), counter-clockwise.
fn square(x: f64, y: f64, side: f64) -> Vec<Point2> {
    vec![
        mm(x, y),
        mm(x + side, y),
        mm(x + side, y + side),
        mm(x, y + side),
    ]
}

/// A circle of `sides` sides about (`cx`, `cy`), counter-clockwise, its
/// first corner half a side past due east.
fn circle(cx: f64, cy: f64, r: f64, sides: usize) -> Vec<Point2> {
    (0..sides)
        .map(|side| {
            let angle = TAU * (side as f64 + 0.5) / sides as f64;
            mm(cx + r * angle.cos(), cy + r * angle.sin())
        })
        .collect()
}

/// A loop once round from its first point, as one stretch.
fn whole(points: &[Point2]) -> Vec<Run> {
    let mut round = points.to_vec();
    round.push(points[0]);
    vec![Run {
        line: arc::Fitted::straight(round),
        flow: flow(),
    }]
}

fn units(mm: f64) -> f64 {
    mm * UNITS_PER_MM
}

/// Length in mm of the stretches of a scarf's way up or way down.
fn stretches_mm(stretches: &[scarf::Stretch]) -> f64 {
    stretches.iter().map(|stretch| long(&stretch.points)).sum()
}

#[test]
fn a_loop_is_cut_into_a_way_up_a_flat_part_and_a_way_down() {
    // A square 15 mm a side is 60 mm round; the slope is 20 mm in ten
    // steps, so no move of it is longer than 2 mm.
    let gap = 0.05;
    let made = sloped(
        whole(&square(0.0, 0.0, 15.0)),
        units(gap),
        20.0,
        units(2.0),
        0.0,
        false,
    );
    let (up, down) = (&made.starts[0], &made.ends[0]);
    assert_eq!(
        (made.starts.len(), made.flat.len(), made.ends.len()),
        (1, 1, 1)
    );
    // Up from the layer beneath to the layer's own height, laying from
    // nothing to the whole line; it starts two seam gaps on.
    assert_eq!((up.from, up.to), ((0.0, 0.0), (1.0, 1.0)));
    assert!((long(&up.points) - (20.0 - 2.0 * gap)).abs() < 1e-5);
    assert!(distance(up.points[0], mm(2.0 * gap, 0.0)) < 1e-5);
    assert_eq!(*up.points.last().unwrap(), mm(15.0, 5.0));
    for pair in up.points.windows(2) {
        assert!(distance(pair[0], pair[1]) <= 2.0 + 1e-9);
    }
    // Round the rest of the loop and back to the seam.
    let flat = &made.flat[0].line.points;
    assert_eq!(
        (flat[0], *flat.last().unwrap()),
        (mm(15.0, 5.0), mm(0.0, 0.0))
    );
    assert!((long(flat) - 40.0).abs() < 1e-5);
    // Then over the way up again, level, from the whole line down to
    // what a seam gap of slope still carries; it stops two gaps short.
    assert_eq!(down.from, (1.0, 1.0));
    assert_eq!(down.to.0, 1.0);
    assert!((down.to.1 - gap / 20.0).abs() < 1e-9, "{:?}", down.to);
    assert_eq!(down.points[0], mm(0.0, 0.0));
    assert!((long(&down.points) - (20.0 - 2.0 * gap)).abs() < 1e-5);

    // With no seam gap nothing is clipped, and the way down ends at nothing.
    let made = sloped(
        whole(&square(0.0, 0.0, 15.0)),
        0.0,
        20.0,
        units(2.0),
        0.0,
        false,
    );
    assert!((stretches_mm(&made.starts) - 20.0).abs() < 1e-5);
    assert!((stretches_mm(&made.ends) - 20.0).abs() < 1e-5);
    assert_eq!(made.ends[0].to, (1.0, 0.0));

    // Started part of the way up, each half of the joint is that much
    // of the line where they meet the seam.
    let made = sloped(
        whole(&square(0.0, 0.0, 15.0)),
        0.0,
        20.0,
        units(2.0),
        0.25,
        false,
    );
    assert_eq!(made.starts[0].from, (0.25, 0.25));
    assert_eq!(
        (made.ends[0].from, made.ends[0].to),
        ((1.0, 0.75), (1.0, 0.0))
    );
}

#[test]
fn a_slope_as_long_as_the_loop_leaves_no_flat_part() {
    let gap = 0.05;
    let made = sloped(
        whole(&square(0.0, 0.0, 5.0)),
        units(gap),
        20.0,
        units(2.0),
        0.0,
        false,
    );
    assert!(made.flat.is_empty());
    assert!((stretches_mm(&made.starts) - (20.0 - 2.0 * gap)).abs() < 1e-5);
    assert!((stretches_mm(&made.ends) - (20.0 - 2.0 * gap)).abs() < 1e-5);
    assert_eq!(made.starts[0].to, (1.0, 1.0));
    // A slope longer than the loop is the loop's length.
    let longer = sloped(
        whole(&square(0.0, 0.0, 5.0)),
        units(gap),
        50.0,
        units(5.0),
        0.0,
        false,
    );
    assert!((stretches_mm(&longer.starts) - (20.0 - 2.0 * gap)).abs() < 1e-5);
}

#[test]
fn an_inner_walls_way_up_starts_two_fifths_of_the_slope_on() {
    let gap = 0.05;
    let inner = |gap: f64, side: f64| {
        sloped(
            whole(&square(0.0, 0.0, side)),
            units(gap),
            20.0,
            units(2.0),
            0.0,
            true,
        )
    };
    let made = inner(gap, 15.0);
    assert!((stretches_mm(&made.starts) - 12.0).abs() < 1e-5);
    assert!(distance(made.starts[0].points[0], mm(8.0, 0.0)) < 1e-5);
    // What is left of it still climbs all the way, from nothing.
    assert_eq!(
        (made.starts[0].from, made.starts[0].to),
        ((0.0, 0.0), (1.0, 1.0))
    );
    // The way down is the whole slope less two seam gaps, as an outer wall's.
    assert!((stretches_mm(&made.ends) - (20.0 - 2.0 * gap)).abs() < 1e-5);
    // Whatever the seam gap: the reference clips nothing here at none.
    assert!((stretches_mm(&inner(0.0, 15.0).starts) - 12.0).abs() < 1e-5);
    // Not where the slope is the whole loop, which would leave two
    // fifths of the wall out of its first time round.
    let small = inner(gap, 5.0);
    assert!(small.flat.is_empty());
    assert!((stretches_mm(&small.starts) - (20.0 - 2.0 * gap)).abs() < 1e-5);
}

#[test]
fn a_few_micrometres_next_to_the_seam_are_dropped() {
    // The seam 10 µm on from a corner, and the loop's last point 10 µm
    // short of the seam: neither is laid as a move of its own.
    let mut round = vec![mm(0.0, 0.0), mm(0.01, 0.0), mm(15.0, 0.0), mm(15.0, 15.0)];
    round.extend([mm(0.0, 15.0), mm(0.0, 0.01), mm(0.0, 0.0)]);
    let runs = vec![Run {
        line: arc::Fitted::straight(round),
        flow: flow(),
    }];
    let made = sloped(runs, 0.0, 20.0, units(2.0), 0.0, false);
    let up = &made.starts[0].points;
    assert_eq!(up[0], mm(0.0, 0.0));
    assert!(distance(up[0], up[1]) > 1.0, "{:?}", &up[..2]);
    let flat = &made.flat[0].line.points;
    let last = flat.len() - 1;
    assert_eq!(flat[last], mm(0.0, 0.0));
    assert_eq!(flat[last - 1], mm(0.0, 15.0));
}

#[test]
fn the_slope_does_not_end_just_short_of_a_point() {
    let run = |points: &[Point2]| {
        vec![Run {
            line: arc::Fitted::straight(points.to_vec()),
            flow: flow(),
        }]
    };
    // A slope of 9.7 mm along a line with a point at 10 mm and the
    // line going straight on: the slope keeps its length, and the flat
    // part goes from its end to the point after, leaving that one out.
    let straight = [mm(0.0, 0.0), mm(10.0, 0.0), mm(20.0, 0.0), mm(20.0, 20.0)];
    let made = sloped(run(&straight), 0.0, 9.7, units(0.97), 0.0, false);
    assert!((stretches_mm(&made.starts) - 9.7).abs() < 1e-5);
    assert_eq!(made.flat[0].line.points[..2], [mm(9.7, 0.0), mm(20.0, 0.0)]);
    // At a corner the slope is carried on to the corner instead.
    let corner = [mm(0.0, 0.0), mm(10.0, 0.0), mm(10.0, 20.0), mm(0.0, 20.0)];
    let made = sloped(run(&corner), 0.0, 9.7, units(0.97), 0.0, false);
    assert!((stretches_mm(&made.starts) - 10.0).abs() < 1e-5);
    assert_eq!(made.starts[0].to, (1.0, 1.0));
    assert_eq!(
        made.flat[0].line.points[..2],
        [mm(10.0, 0.0), mm(10.0, 20.0)]
    );
    // Further than half a step from the next point it ends where it ends.
    let made = sloped(run(&corner), 0.0, 9.0, units(0.9), 0.0, false);
    assert!((stretches_mm(&made.starts) - 9.0).abs() < 1e-5);
    assert_eq!(made.flat[0].line.points[..2], [mm(9.0, 0.0), mm(10.0, 0.0)]);
}

#[test]
fn a_slope_runs_on_from_one_width_of_a_loop_into_the_next() {
    // A loop in three stretches of 8 mm, each its own width, and a
    // slope of 20 mm: the first two are all slope and the third is cut.
    let narrow = Flow::new(0.4, 0.2, 0.4).unwrap();
    let runs: Vec<Run> = [(0.0, flow()), (8.0, narrow), (16.0, flow())]
        .into_iter()
        .map(|(x, flow)| Run {
            line: arc::Fitted::straight(vec![mm(x, 0.0), mm(x + 8.0, 0.0)]),
            flow,
        })
        .collect();
    let made = sloped(runs, 0.0, 20.0, units(2.0), 0.0, false);
    assert_eq!(
        (made.starts.len(), made.flat.len(), made.ends.len()),
        (3, 1, 3)
    );
    let shares: Vec<(f64, f64)> = (made.starts.iter())
        .map(|start| (start.from.0, start.to.0))
        .collect();
    let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
    assert!(
        near(shares[0].0, 0.0) && near(shares[0].1, 0.4),
        "{shares:?}"
    );
    assert!(
        near(shares[1].0, 0.4) && near(shares[1].1, 0.8),
        "{shares:?}"
    );
    assert!(
        near(shares[2].0, 0.8) && near(shares[2].1, 1.0),
        "{shares:?}"
    );
    assert_eq!(made.starts[1].flow, narrow);
    // The way down lays what the way up left: together, the whole line.
    for (up, down) in made.starts.iter().zip(&made.ends) {
        assert!(near(up.from.1 + down.from.1, 1.0) && near(up.to.1 + down.to.1, 1.0));
        assert_eq!(up.flow, down.flow);
    }
    assert_eq!(made.flat[0].line.points, [mm(20.0, 0.0), mm(24.0, 0.0)]);
}

#[test]
fn a_slope_of_any_step_is_cut_without_end() {
    // Steps finer than a coordinate can tell apart still come to an end.
    let made = sloped(whole(&square(0.0, 0.0, 1.0)), 0.0, 0.00001, 0.1, 0.0, false);
    assert!(made.starts.iter().all(|start| start.points.len() < 100));
    assert_eq!(
        sloped(Vec::new(), 0.0, 20.0, units(2.0), 0.0, false),
        Sloped::default()
    );
}

#[test]
fn a_loop_with_a_sharp_corner_is_not_smooth() {
    let round = circle(0.0, 0.0, 10.0, 128);
    let limit = 155f64.to_radians();
    assert!(is_smooth(&round, limit, 0.4));
    assert!(!is_smooth(&square(0.0, 0.0, 20.0), limit, 0.4));
    // Twelve sides turn 30 degrees at each corner, 150 between them:
    // sharp at 155, not at 140, from wherever the loop is taken to start.
    let dented: Vec<Point2> = circle(0.0, 0.0, 10.0, 12);
    for start in 0..12 {
        let mut turned = dented.clone();
        turned.rotate_left(start);
        assert!(!is_smooth(&turned, limit, 0.4));
        assert!(is_smooth(&turned, 140f64.to_radians(), 0.4));
    }
    // Two points are no loop.
    assert!(!is_smooth(&[mm(0.0, 0.0), mm(1.0, 0.0)], limit, 0.4));
}

fn wall(points: Vec<Point2>, depth: usize, is_hole: bool) -> ExtrusionLoop {
    ExtrusionLoop {
        polygon: Polygon { points },
        role: if depth == 0 {
            Role::ExternalPerimeter
        } else {
            Role::Perimeter
        },
        is_hole,
        depth,
        flow: flow(),
        widths: Vec::new(),
        clockwise: false,
        circle_compensation: false,
    }
}

/// Walls at 60 and 80 mm/s that start at the nearest corner, with
/// nothing else that would change how a wall is laid, and a scarf on
/// the loops round the part.
fn settings() -> PrintSettings {
    PrintSettings {
        seam: Seam::Nearest,
        seam_gap: 0.05,
        lift: Lift::Normal,
        z_hop: 0.0,
        wipe: false,
        slow_down_for_layer_cooling: false,
        outer_wall_speed: 60.0,
        inner_wall_speed: 80.0,
        initial_layer_speed: 20.0,
        filament_max_volumetric_speed: 0.0,
        enable_overhang_speed: false,
        detect_overhang_wall: false,
        overhang_fan: None,
        small_perimeter_threshold: 0.0,
        scarf: Scarf {
            kind: ScarfSeam::External,
            ..Scarf::default()
        },
        ..PrintSettings::default()
    }
}

/// Two layers of these walls, 0.2 mm each, every wall an island, the
/// second layer over `below`.
fn printed(loops: &[ExtrusionLoop], below: &[ExPolygon], settings: &PrintSettings) -> String {
    let islands: Vec<Range<usize>> = (0..loops.len()).map(|at| at..at + 1).collect();
    let layer = |z: f64, below| Layer {
        region: None,
        z,
        height: 0.2,
        filament: 0,
        ram: None,
        enter: &[],
        wipe: None,
        tower: &[],
        tower_z: None,
        tower_first: false,
        around: &[],
        support: &[],
        loops,
        islands: &islands,
        paths: &[],
        shelter: &[],
        below,
    };
    let mut out = Vec::new();
    write(&mut out, &[layer(0.2, &[]), layer(0.4, below)], settings).unwrap();
    String::from_utf8(out).unwrap()
}

fn word(line: &str, word: char) -> Option<f64> {
    line.split(' ')
        .find_map(|part| part.strip_prefix(word)?.parse().ok())
}

/// One extruding move of the second layer.
#[derive(Clone, Copy, Debug)]
struct Laid {
    to: (f64, f64),
    /// The height the move says it ends at, if it says.
    z: Option<f64>,
    /// The height the nozzle is at once it is made.
    at: f64,
    length: f64,
    fed: f64,
    /// In mm/s.
    speed: f64,
    arc: bool,
    feature: &'static str,
}

impl Laid {
    fn per_mm(&self) -> f64 {
        self.fed / self.length
    }
}

/// The extruding moves of the second layer, and the lines between the
/// first of them and the last that are no such move.
fn second_layer(text: &str) -> (Vec<Laid>, Vec<String>) {
    let names = ["Outer wall", "Inner wall", "Overhang wall"];
    let (mut moves, mut others, mut between) = (Vec::new(), Vec::new(), Vec::new());
    let (mut layer, mut at, mut height, mut speed, mut feature) =
        (0, None::<(f64, f64)>, 0.0, 0.0, "");
    for line in text.lines() {
        if line.starts_with(";LAYER_CHANGE") {
            layer += 1;
        }
        if let Some(name) = line.strip_prefix(";TYPE:") {
            feature = names.iter().find(|known| **known == name).unwrap_or(&"");
        }
        let arc = line.starts_with("G2 ") || line.starts_with("G3 ");
        if !(arc || line.starts_with("G1 ")) {
            if layer == 2 && !moves.is_empty() && !line.starts_with(';') {
                between.push(line.to_owned());
            }
            continue;
        }
        speed = word(line, 'F').map_or(speed, |f| f / 60.0);
        height = word(line, 'Z').unwrap_or(height);
        let to = word(line, 'X').zip(word(line, 'Y'));
        let fed = word(line, 'E').filter(|fed| *fed >= 0.0);
        if let (2, Some((x, y)), Some(to), Some(fed)) = (layer, at, to, fed) {
            others.append(&mut between);
            moves.push(Laid {
                to,
                z: word(line, 'Z'),
                at: height,
                length: (to.0 - x).hypot(to.1 - y),
                fed,
                speed,
                arc,
                feature,
            });
        } else if layer == 2 && !moves.is_empty() {
            between.push(line.to_owned());
        }
        at = to.or(at);
    }
    (moves, others)
}

/// The way up and the way down of the only scarf among `moves`: the
/// moves that say a height, below the layer's and at it.
fn joint(moves: &[Laid]) -> (Vec<Laid>, Vec<Laid>) {
    let sloped = moves.iter().filter(|laid| laid.z.is_some());
    let mut up: Vec<Laid> = Vec::new();
    let mut down = Vec::new();
    for laid in sloped {
        // The way up ends at the layer's height; the way down stays at it.
        let climbing = up.last().is_none_or(|last: &Laid| last.at < 0.4);
        if climbing && down.is_empty() {
            up.push(*laid);
        } else {
            down.push(*laid);
        }
    }
    (up, down)
}

/// What a whole line of [`flow`] feeds for each mm, as [`settings`] have it.
fn line_per_mm(settings: &PrintSettings) -> f64 {
    flow().mm3_per_mm() * settings.filament_flow_ratio * settings.print_flow_ratio
        / (PI * (settings.filament_diameter / 2.0).powi(2))
}

#[test]
fn a_round_wall_is_joined_on_a_slope() {
    let loops = [wall(circle(50.0, 50.0, 10.0, 128), 0, false)];
    let s = settings();
    let text = printed(&loops, &[], &s);
    let (moves, others) = second_layer(&text);
    let (up, down) = joint(&moves);
    let whole = line_per_mm(&s);
    let round = long(&whole_round(&loops[0]));

    // Nothing but moves from the first of the wall to the last: the
    // nozzle goes down to the foot of the slope before it, and is back
    // at the layer's height with no move of its own.
    assert!(others.is_empty(), "{others:?}");
    let layer: Vec<&str> = (text.lines())
        .skip_while(|line| !line.starts_with(";Z:0.400"))
        .collect();
    let first = (layer.iter())
        .position(|line| line.starts_with("G1 X") && line.contains(" Z") && line.contains(" E"))
        .unwrap();
    let before: Vec<&&str> = (layer[..first].iter())
        .filter(|line| line.starts_with("G1 "))
        .collect();
    assert_eq!(**before.last().unwrap(), "G1 Z0.200", "{before:?}");

    // The way up: 20 mm less two seam gaps, climbing all the way from
    // the top of the layer beneath, never under it, laying more as it goes.
    assert!((up.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
    assert!(up.len() >= 10);
    assert!(up.windows(2).all(|pair| pair[0].at <= pair[1].at));
    assert!(up.iter().all(|laid| laid.at >= 0.2 && laid.at <= 0.4));
    assert!(up[0].at < 0.21 && up[up.len() - 1].at == 0.4, "{up:?}");
    assert!(
        up.windows(2)
            .all(|pair| pair[0].per_mm() < pair[1].per_mm() + 1e-4)
    );
    assert!(up[0].per_mm() < 0.1 * whole);
    assert!((up[up.len() - 1].per_mm() / whole - 1.0).abs() < 0.02);
    // Then the rest of the loop at the layer's height, saying no height.
    let flat: Vec<&Laid> = moves.iter().filter(|laid| laid.z.is_none()).collect();
    assert!(flat.iter().all(|laid| laid.at == 0.4));
    assert!((flat.iter().map(|laid| laid.length).sum::<f64>() - (round - 20.0)).abs() < 0.01);
    assert!(
        flat.iter()
            .all(|laid| (laid.per_mm() / whole - 1.0).abs() < 0.02)
    );
    // And the way down over the way up, level, laying less as it goes.
    assert!((down.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
    assert!(down.iter().all(|laid| laid.z == Some(0.4)));
    assert!(
        down.windows(2)
            .all(|pair| pair[0].per_mm() + 1e-4 > pair[1].per_mm())
    );
    assert!(down[down.len() - 1].per_mm() < 0.1 * whole);
    // Between them the two halves lay one whole line along the joint,
    // and the loop is laid once and a slope's length round.
    let fed: f64 = up.iter().chain(&down).map(|laid| laid.fed).sum();
    assert!((fed / (whole * 20.0) - 1.0).abs() < 0.01, "{fed}");
    let all: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((all - (round + 20.0 - 0.2)).abs() < 0.02, "{all} {round}");
    // All of it is the outer wall, at the outer wall's speed.
    assert!(
        moves
            .iter()
            .all(|laid| laid.feature == "Outer wall" && laid.speed == 60.0)
    );

    // The first layer has none: its wall stops a seam gap short.
    let first: Vec<&str> = text
        .lines()
        .take_while(|line| !line.starts_with(";Z:0.400"))
        .filter(|line| line.starts_with("G1 X") && line.contains(" E"))
        .collect();
    assert!(first.iter().all(|line| !line.contains(" Z")));
}

/// A wall's loop once round, its first point given again.
fn whole_round(wall: &ExtrusionLoop) -> Vec<Point2> {
    let mut round = wall.polygon.points.clone();
    round.push(round[0]);
    round
}

/// How many of the second layer's moves are on a scarf.
fn sloped_moves(text: &str) -> usize {
    let (moves, _) = second_layer(text);
    moves.iter().filter(|laid| laid.z.is_some()).count()
}

#[test]
fn which_loops_are_joined_so_is_as_asked() {
    let round = || circle(50.0, 50.0, 10.0, 128);
    let outer = [wall(round(), 0, false)];
    let hole = [wall(round(), 0, true)];
    let inner = [wall(round(), 1, false)];
    let with = |kind, inner_walls| PrintSettings {
        scarf: Scarf {
            kind,
            inner_walls,
            ..Scarf::default()
        },
        ..settings()
    };
    // None leaves every file as it was without any of this.
    let none = printed(&outer, &[], &with(ScarfSeam::None, true));
    assert_eq!(sloped_moves(&none), 0);
    assert!(
        !none
            .lines()
            .any(|line| line.starts_with("G1 X") && line.contains(" Z") && line.contains(" E"))
    );
    assert!(sloped_moves(&printed(&outer, &[], &with(ScarfSeam::External, false))) > 0);
    // Round a hole only when holes are asked for too.
    assert_eq!(
        sloped_moves(&printed(&hole, &[], &with(ScarfSeam::External, false))),
        0
    );
    assert!(sloped_moves(&printed(&hole, &[], &with(ScarfSeam::All, false))) > 0);
    // Inner walls only when they are asked for.
    assert_eq!(
        sloped_moves(&printed(&inner, &[], &with(ScarfSeam::All, false))),
        0
    );
    assert!(sloped_moves(&printed(&inner, &[], &with(ScarfSeam::All, true))) > 0);

    // A slope of no length is a plain seam: the loop stops a seam gap
    // short of its start, where the reference would close it.
    let no_length = PrintSettings {
        scarf: Scarf {
            min_length: 0.0,
            ..settings().scarf
        },
        ..settings()
    };
    let text = printed(&outer, &[], &no_length);
    let (moves, _) = second_layer(&text);
    assert!(moves.iter().all(|laid| laid.z.is_none()));
    let all: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((all - (long(&whole_round(&outer[0])) - 0.05)).abs() < 0.01);
    let plain = PrintSettings {
        scarf: Scarf::default(),
        ..settings()
    };
    assert_eq!(text, printed(&outer, &[], &plain));
}

#[test]
fn the_slope_is_as_long_as_high_and_as_fine_as_asked() {
    let loops = [wall(circle(50.0, 50.0, 10.0, 128), 0, false)];
    let with = |scarf: Scarf| {
        let text = printed(
            &loops,
            &[],
            &PrintSettings {
                scarf,
                ..settings()
            },
        );
        joint(&second_layer(&text).0)
    };
    let usual = settings().scarf;
    let length = |moves: &[Laid]| moves.iter().map(|laid| laid.length).sum::<f64>();

    // Its length, and the whole loop.
    let (up, down) = with(Scarf {
        min_length: 8.0,
        ..usual
    });
    assert!((length(&up) - 7.9).abs() < 0.01 && (length(&down) - 7.9).abs() < 0.01);
    let (up, down) = with(Scarf {
        entire_loop: true,
        ..usual
    });
    let round = long(&whole_round(&loops[0]));
    assert!(
        (length(&up) - (round - 0.1)).abs() < 0.01,
        "{}",
        length(&up)
    );
    assert!((length(&down) - (round - 0.1)).abs() < 0.01);

    // Its start: 0.05 mm up, a quarter of the layer; and as a percentage.
    for start_height in [FloatOrPercent::Value(0.05), FloatOrPercent::Percent(25.0)] {
        let (up, down) = with(Scarf {
            start_height,
            ..usual
        });
        assert!(up.iter().all(|laid| laid.at >= 0.25), "{up:?}");
        assert!(up[0].at < 0.26);
        let whole = line_per_mm(&settings());
        assert!((up[0].per_mm() / whole - 0.25).abs() < 0.02);
        assert!((down[0].per_mm() / whole - 0.75).abs() < 0.02);
    }
    // A start at the layer's height or over is just under it.
    let (up, _) = with(Scarf {
        start_height: FloatOrPercent::Value(0.3),
        ..usual
    });
    assert!(
        up.iter().all(|laid| laid.at >= 0.398 && laid.at <= 0.4),
        "{up:?}"
    );

    // Its steps: 20 mm in 100 is moves of no more than 0.2 mm, where
    // the wall's own sides are half a millimetre.
    let (up, _) = with(Scarf {
        steps: 100,
        ..usual
    });
    assert!(up.iter().all(|laid| laid.length <= 0.2 + 1e-3));
    assert!(up.len() >= 100);
}

#[test]
fn the_joint_has_its_own_speed_and_flow() {
    let loops = [wall(circle(50.0, 50.0, 10.0, 128), 0, false)];
    let with = |scarf: Scarf, change: &dyn Fn(&mut PrintSettings)| {
        let mut s = PrintSettings {
            scarf,
            ..settings()
        };
        change(&mut s);
        second_layer(&printed(&loops, &[], &s)).0
    };
    let usual = settings().scarf;
    let speeds = |moves: &[Laid]| {
        let on = |sloped: bool| {
            let mut found: Vec<f64> = (moves.iter())
                .filter(|laid| laid.z.is_some() == sloped)
                .map(|laid| laid.speed)
                .collect();
            found.dedup();
            found
        };
        (on(true), on(false))
    };
    // A speed of its own, where that is the slower.
    let slow = Scarf {
        joint_speed: FloatOrPercent::Value(20.0),
        ..usual
    };
    assert_eq!(speeds(&with(slow, &|_| {})), (vec![20.0], vec![60.0]));
    let fast = Scarf {
        joint_speed: FloatOrPercent::Value(200.0),
        ..usual
    };
    assert_eq!(speeds(&with(fast, &|_| {})), (vec![60.0], vec![60.0]));
    // A percentage is of the wall's speed.
    let half = Scarf {
        joint_speed: FloatOrPercent::Percent(50.0),
        ..usual
    };
    assert_eq!(speeds(&with(half, &|_| {})), (vec![30.0], vec![60.0]));
    // A small loop's speed does not let the joint go faster than its
    // own (the reference leaves the joint's speed out there).
    let small = |s: &mut PrintSettings| {
        s.small_perimeter_threshold = 15.0;
        s.small_perimeter_speed = FloatOrPercent::Value(40.0);
    };
    assert_eq!(speeds(&with(slow, &small)), (vec![20.0], vec![40.0]));
    // On a small loop too a percentage is of the wall's own speed, not
    // of the small loop's: half of 60, not half of 40.
    assert_eq!(speeds(&with(half, &small)), (vec![30.0], vec![40.0]));
    // Half of a 100 mm/s wall that is a small loop at 60 is 50.
    let fast_wall = |s: &mut PrintSettings| {
        s.outer_wall_speed = 100.0;
        s.small_perimeter_threshold = 15.0;
        s.small_perimeter_speed = FloatOrPercent::Value(60.0);
    };
    assert_eq!(speeds(&with(half, &fast_wall)), (vec![50.0], vec![60.0]));
    // And never faster than the small loop goes: nine tenths of 100 is
    // more than 60, and the joint stays at 60.
    let most = Scarf {
        joint_speed: FloatOrPercent::Percent(90.0),
        ..usual
    };
    assert_eq!(speeds(&with(most, &fast_wall)), (vec![60.0], vec![60.0]));
    // A speed in mm/s is what it was.
    assert_eq!(speeds(&with(slow, &fast_wall)), (vec![20.0], vec![60.0]));
    // Nothing, or less, is no speed of its own (the settings turn a 0
    // into the wall's speed and say so; this is for what gets by).
    let none = Scarf {
        joint_speed: FloatOrPercent::Value(0.0),
        ..usual
    };
    assert_eq!(speeds(&with(none, &fast_wall)), (vec![60.0], vec![60.0]));

    // More or less plastic along the joint, and none more elsewhere.
    let rich = Scarf {
        joint_flow_ratio: 1.2,
        ..usual
    };
    let (plain, more) = (with(usual, &|_| {}), with(rich, &|_| {}));
    for (plain, more) in plain.iter().zip(&more) {
        let share = if plain.z.is_some() { 1.2 } else { 1.0 };
        assert!(
            (more.fed - share * plain.fed).abs() < 2e-5,
            "{plain:?} {more:?}"
        );
    }
    // The hot end's limit is reckoned with it: 4 mm3/s through this
    // line is about 49 mm/s, and through a fifth more of it about 41.
    let choked = |s: &mut PrintSettings| s.filament_max_volumetric_speed = 4.0;
    let (on, off) = speeds(&with(rich, &choked));
    assert!(
        on[0] > 40.0 && on[0] < 42.0 && off[0] > 48.0 && off[0] < 50.0,
        "{on:?} {off:?}"
    );
}

fn plate(x: f64, y: f64, side: f64) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: square(x, y, side),
        },
        holes: Vec::new(),
    }
}

fn disc(cx: f64, cy: f64, r: f64) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: circle(cx, cy, r, 128),
        },
        holes: Vec::new(),
    }
}

#[test]
fn a_conditional_scarf_wants_a_smooth_loop_resting_on_the_layer_beneath() {
    let conditional = |overhang_threshold| PrintSettings {
        scarf: Scarf {
            conditional: true,
            overhang_threshold,
            ..settings().scarf
        },
        ..settings()
    };
    let s = conditional(0.4);
    // A round wall 0.45 mm wide whose middle is 10 mm out, over a disc
    // of this radius: resting on it, or hanging out.
    let round = [wall(circle(50.0, 50.0, 10.0, 128), 0, false)];
    let over = |radius: f64, s: &PrintSettings| {
        sloped_moves(&printed(&round, &[disc(50.0, 50.0, radius)], s)) > 0
    };
    assert!(over(10.225, &s));
    // The middle of the line may be short of the edge beneath by no
    // more than the threshold less four tenths of the width: at 40 %
    // not at all, so a wall whose middle is over air has none.
    assert!(over(10.01, &s));
    assert!(!over(9.95, &s));
    // At 60 % it may be 0.09 mm out, and at none it may be anywhere.
    assert!(over(9.95, &conditional(0.6)));
    assert!(!over(9.85, &conditional(0.6)));
    assert!(over(9.0, &conditional(0.0)));
    // With nothing beneath at all there is nothing to rest on.
    assert_eq!(sloped_moves(&printed(&round, &[], &s)), 0);
    assert!(sloped_moves(&printed(&round, &[], &conditional(0.0))) > 0);

    // A square has corners, however well it rests.
    let cornered = [wall(square(40.0, 40.0, 20.0), 0, false)];
    let under = [plate(30.0, 30.0, 40.0)];
    assert_eq!(sloped_moves(&printed(&cornered, &under, &s)), 0);
    // Unless corners that sharp are let through, or nothing is conditional.
    let lax = PrintSettings {
        scarf: Scarf {
            angle_threshold: 80.0,
            ..s.scarf
        },
        ..s.clone()
    };
    assert!(sloped_moves(&printed(&cornered, &under, &lax)) > 0);
    assert!(sloped_moves(&printed(&cornered, &under, &settings())) > 0);

    // An inner wall is asked about the outer wall's seam it starts
    // beside: 0.45 mm further in, it rests where the outer wall hangs out.
    let pair = [
        wall(circle(50.0, 50.0, 10.0, 128), 0, false),
        wall(circle(50.0, 50.0, 9.55, 128), 1, false),
    ];
    let both = PrintSettings {
        scarf: Scarf {
            inner_walls: true,
            ..s.scarf
        },
        ..s
    };
    let islands = [0..2];
    let print = |below: &[ExPolygon]| {
        let layer = |z: f64, below| Layer {
            region: None,
            z,
            height: 0.2,
            filament: 0,
            ram: None,
            enter: &[],
            wipe: None,
            tower: &[],
            tower_z: None,
            tower_first: false,
            around: &[],
            support: &[],
            loops: &pair,
            islands: &islands,
            paths: &[],
            shelter: &[],
            below,
        };
        let mut out = Vec::new();
        write(&mut out, &[layer(0.2, &[]), layer(0.4, below)], &both).unwrap();
        let text = String::from_utf8(out).unwrap();
        let (moves, _) = second_layer(&text);
        let on =
            |feature: &str| (moves.iter()).any(|laid| laid.z.is_some() && laid.feature == feature);
        (on("Outer wall"), on("Inner wall"))
    };
    assert_eq!(print(&[disc(50.0, 50.0, 10.225)]), (true, true));
    assert_eq!(print(&[disc(50.0, 50.0, 9.95)]), (false, false));
}

#[test]
fn a_compensated_circles_walls_are_joined_so_whatever_else_is_asked() {
    let circle_wall = |depth: usize, is_hole: bool| ExtrusionLoop {
        circle_compensation: true,
        ..wall(circle(50.0, 50.0, 3.0, 64), depth, is_hole)
    };
    let s = PrintSettings {
        scarf: Scarf::default(),
        circle_compensation_speed: 0.0,
        ..settings()
    };
    assert_eq!(s.scarf.kind, ScarfSeam::None);
    // Outer and inner, round the part and round a hole.
    for loops in [
        [circle_wall(0, true)],
        [circle_wall(1, true)],
        [circle_wall(0, false)],
    ] {
        let text = printed(&loops, &[], &s);
        let (moves, _) = second_layer(&text);
        let (up, down) = joint(&moves);
        // The loop is under 20 mm round, so all of it is slope: laid
        // twice, less two seam gaps each time.
        let round = long(&whole_round(&loops[0]));
        let length = |moves: &[Laid]| moves.iter().map(|laid| laid.length).sum::<f64>();
        assert!(
            (length(&up) - (round - 0.1)).abs() < 0.01,
            "{}",
            length(&up)
        );
        assert!((length(&down) - (round - 0.1)).abs() < 0.01);
        assert_eq!(moves.len(), up.len() + down.len());
        // Not on the first layer.
        assert!(
            (text.lines())
                .take_while(|line| !line.starts_with(";Z:0.400"))
                .all(|line| !(line.starts_with("G1 X")
                    && line.contains(" Z")
                    && line.contains(" E")))
        );
    }
    // A wall like any other with the option off, or when it is no
    // such circle.
    let off = PrintSettings {
        scarf: Scarf {
            on_circles: false,
            ..Scarf::default()
        },
        ..s.clone()
    };
    assert_eq!(
        sloped_moves(&printed(&[circle_wall(0, true)], &[], &off)),
        0
    );
    let plain = [wall(circle(50.0, 50.0, 3.0, 64), 0, true)];
    assert_eq!(sloped_moves(&printed(&plain, &[], &s)), 0);
    // Nor where any of it hangs clear of the layer beneath.
    let hanging = PrintSettings {
        detect_overhang_wall: true,
        ..s
    };
    let under = [plate(40.0, 40.0, 10.0)];
    assert_eq!(
        sloped_moves(&printed(&[circle_wall(0, false)], &under, &hanging)),
        0
    );
    assert!(
        sloped_moves(&printed(
            &[circle_wall(0, false)],
            &[disc(50.0, 50.0, 5.0)],
            &hanging
        )) > 0
    );
}

#[test]
fn a_slope_goes_on_over_air_as_an_overhang_wall() {
    let s = PrintSettings {
        detect_overhang_wall: true,
        bridge_speed: 25.0,
        ..settings()
    };
    // A square wall 20 mm a side; the nearest corner to where the
    // nozzle starts is its first, at (40, 40), and it goes along y = 40
    // first. Beneath it a plate with a notch cut from x = 46 to 50, so
    // that stretch of the first side has nothing under it.
    let loops = [wall(square(40.0, 40.0, 20.0), 0, false)];
    let notched = |from: f64, to: f64| {
        vec![ExPolygon {
            contour: Polygon {
                points: vec![
                    mm(30.0, 30.0),
                    mm(from, 30.0),
                    mm(from, 45.0),
                    mm(to, 45.0),
                    mm(to, 30.0),
                    mm(70.0, 30.0),
                    mm(70.0, 70.0),
                    mm(30.0, 70.0),
                ],
            },
            holes: Vec::new(),
        }]
    };
    let text = printed(&loops, &notched(46.0, 50.0), &s);
    let (moves, others) = second_layer(&text);
    assert!(others.is_empty(), "{others:?}");
    let (up, down) = joint(&moves);
    // The way up climbs straight through the stretch over air, which
    // is laid as an overhang wall, at the bridge's speed and with the
    // bridge's line, thinned as the slope thins it.
    assert!(up.windows(2).all(|pair| pair[0].at <= pair[1].at));
    let over: Vec<&Laid> = up
        .iter()
        .filter(|laid| laid.feature == "Overhang wall")
        .collect();
    assert!(!over.is_empty());
    assert!(
        over.iter()
            .all(|laid| laid.speed == 25.0 && laid.to.0 > 46.0 && laid.to.0 < 50.5)
    );
    assert!(
        over.iter().all(|laid| laid.at > 0.25 && laid.at < 0.31),
        "{over:?}"
    );
    assert!((up.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
    // So is the way down.
    assert!(down.iter().any(|laid| laid.feature == "Overhang wall"));
    assert!(down.iter().all(|laid| laid.z == Some(0.4)));
    assert!((down.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
}

/// Where the second layer's first extruding move starts.
fn second_layer_start(text: &str) -> (f64, f64) {
    let mut at = (f64::NAN, f64::NAN);
    for line in text
        .lines()
        .skip_while(|line| !line.starts_with(";Z:0.400"))
    {
        if !line.starts_with("G1 ") {
            continue;
        }
        let to = word(line, 'X').zip(word(line, 'Y'));
        if to.is_some() && word(line, 'E').is_some_and(|fed| fed > 0.0) {
            break;
        }
        at = to.unwrap_or(at);
    }
    at
}

#[test]
fn a_seam_keeps_off_what_hangs_out_and_a_wall_all_over_air_has_no_scarf() {
    let s = PrintSettings {
        detect_overhang_wall: true,
        bridge_speed: 25.0,
        ..settings()
    };
    // A square wall 20 mm a side whose first corner, at (40, 40), is the
    // nearest to where the nozzle is. Beneath it a plate with a notch cut
    // from x = 38 to 44 up to y = 45: that corner and the first 4 mm of
    // the wall have nothing under them.
    let loops = [wall(square(40.0, 40.0, 20.0), 0, false)];
    let notched = |plate: (f64, f64)| {
        vec![ExPolygon {
            contour: Polygon {
                points: vec![
                    mm(plate.0, plate.0),
                    mm(38.0, plate.0),
                    mm(38.0, 45.0),
                    mm(44.0, 45.0),
                    mm(44.0, plate.0),
                    mm(plate.1, plate.0),
                    mm(plate.1, plate.1),
                    mm(plate.0, plate.1),
                ],
            },
            holes: Vec::new(),
        }]
    };
    let under = notched((30.0, 70.0));
    // The seam is not put at the corner over the notch, with a scarf or
    // without one (the reference's seams keep off what hangs out before
    // anything else is weighed, SeamPlacer.cpp `is_first_better`): it
    // goes to a corner that rests, one of the two nearest.
    let plain = PrintSettings {
        scarf: Scarf::default(),
        ..s.clone()
    };
    let text = printed(&loops, &under, &plain);
    let (moves, _) = second_layer(&text);
    assert_eq!(second_layer_start(&text), (40.0, 60.0));
    assert_eq!(moves[0].feature, "Outer wall");
    assert!(moves.iter().all(|laid| laid.z.is_none()));

    // So the wall is joined with a scarf there, and what is over the
    // notch is an overhang wall each time it is passed.
    let text = printed(&loops, &under, &s);
    let (moves, others) = second_layer(&text);
    assert!(others.is_empty(), "{others:?}");
    // (The way up starts short of the seam by the gap left there.)
    assert_eq!(second_layer_start(&text), (40.0, 59.9));
    let (up, down) = joint(&moves);
    let length = |moves: &[Laid]| moves.iter().map(|laid| laid.length).sum::<f64>();
    assert!((length(&up) - 19.9).abs() < 0.01);
    assert!((length(&down) - 19.9).abs() < 0.01);
    assert!(up[0].at < 0.22 && up.windows(2).all(|pair| pair[0].at <= pair[1].at));
    assert!(down.iter().all(|laid| laid.z == Some(0.4)));
    assert!((length(&moves) - (80.0 + 20.0 - 0.2)).abs() < 0.02);
    let over: Vec<Laid> = (moves.iter().copied())
        .filter(|laid| laid.feature == "Overhang wall")
        .collect();
    assert!(!over.is_empty());
    assert!(over.iter().all(|laid| laid.speed == 25.0));

    // A wall with no corner that rests starts at the one that hangs out
    // least, over air, and a wall that starts over air has no scarf:
    // over a plate that stops short of it all round it is laid as it is
    // with no scarf.
    let small = notched((41.0, 59.0));
    let text = printed(&loops, &small, &s);
    let (moves, _) = second_layer(&text);
    assert_eq!(moves[0].feature, "Overhang wall");
    assert_eq!(sloped_moves(&text), 0);
    assert_eq!(text, printed(&loops, &small, &plain));
}

#[test]
fn arcs_are_kept_off_the_slope_and_on_the_flat_part() {
    let loops = [wall(circle(50.0, 50.0, 10.0, 128), 0, false)];
    let s = PrintSettings {
        arc_fitting: true,
        resolution: 0.012,
        ..settings()
    };
    let text = printed(&loops, &[], &s);
    let (moves, others) = second_layer(&text);
    assert!(others.is_empty(), "{others:?}");
    // The way up and the way down are straight moves through the points
    // the arcs are drawn by (GCode.cpp `_extrude`: "use G1 if ... we
    // are doing sloped extrusion"); the rest of the loop keeps its arcs.
    assert!(
        moves
            .iter()
            .filter(|laid| laid.z.is_some())
            .all(|laid| !laid.arc)
    );
    assert!(moves.iter().any(|laid| laid.arc && laid.z.is_none()));
    let (up, down) = joint(&moves);
    assert!((up.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.02);
    assert!((down.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.02);
    assert!(up.len() >= 10 && up[up.len() - 1].at == 0.4);
    assert!(moves.iter().all(|laid| laid.at >= 0.2 && laid.at <= 0.4));
}

#[test]
fn a_wall_of_changing_width_is_joined_on_a_slope_too() {
    // A square whose width goes from 0.4 mm at two corners to 0.6 at
    // the others.
    let mut varied = wall(square(40.0, 40.0, 20.0), 0, false);
    varied.widths = vec![400_000, 600_000, 400_000, 600_000];
    let text = printed(&[varied], &[], &settings());
    let (moves, others) = second_layer(&text);
    assert!(
        others.iter().all(|line| !line.starts_with('G')),
        "{others:?}"
    );
    let (up, down) = joint(&moves);
    assert!((up.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
    assert!((down.iter().map(|laid| laid.length).sum::<f64>() - 19.9).abs() < 0.01);
    assert!(up.windows(2).all(|pair| pair[0].at <= pair[1].at));
    assert!(up[0].at < 0.23 && up[up.len() - 1].at == 0.4);
    // Several widths are said along the slope.
    let widths = (text.lines())
        .skip_while(|line| !line.starts_with(";Z:0.400"))
        .filter(|line| line.starts_with(";WIDTH:"))
        .count();
    assert!(widths > 3, "{widths}");
    let all: f64 = moves.iter().map(|laid| laid.length).sum();
    assert!((all - (80.0 + 20.0 - 0.2)).abs() < 0.02, "{all}");
}

#[test]
fn the_nozzle_comes_down_from_a_lift_to_the_foot_of_the_slope_and_wipes_on_round() {
    // Two walls far enough apart for the move between them to retract
    // and lift.
    let loops = [
        wall(circle(50.0, 50.0, 10.0, 128), 0, false),
        wall(circle(120.0, 50.0, 10.0, 128), 0, false),
    ];
    let s = PrintSettings {
        z_hop: 0.4,
        wipe: true,
        wipe_distance: 2.0,
        wipe_speed: Some(50.0),
        retraction_length: 0.8,
        ..settings()
    };
    let text = printed(&loops, &[], &s);
    let layer: Vec<&str> = (text.lines())
        .skip_while(|line| !line.starts_with(";Z:0.400"))
        .collect();
    // After the travel at the lifted height, straight down to the top
    // of the layer beneath, then the filament back, then the slope.
    let down = (layer.iter())
        .rposition(|line| line.starts_with("G1 Z0.200"))
        .expect("no move down to the foot of the slope");
    assert!(
        layer[down - 2].starts_with("G1 Z0.800"),
        "{}",
        layer[down - 2]
    );
    assert!(layer[down - 1].starts_with("G1 X"), "{}", layer[down - 1]);
    assert!(
        layer[down + 1].starts_with("G1 E0.8"),
        "{}",
        layer[down + 1]
    );
    let next = (layer[down + 2..].iter())
        .find(|line| line.starts_with("G1 "))
        .unwrap();
    assert!(next.contains(" Z0.2") && next.contains(" E"), "{next}");
    // The wipe after the first wall sets off along the flat part, on
    // from where the slope came up, not back over the way down.
    let (moves, _) = second_layer(&text);
    let top = moves.iter().position(|laid| laid.z == Some(0.4)).unwrap();
    let (from, on) = (moves[top].to, moves[top + 1].to);
    let wipe = layer
        .iter()
        .position(|line| *line == ";WIPE_START")
        .unwrap();
    let to = (
        word(layer[wipe + 2], 'X').unwrap(),
        word(layer[wipe + 2], 'Y').unwrap(),
    );
    assert!(
        (to.0 - on.0).hypot(to.1 - on.1) < 1e-3,
        "{to:?} {from:?} {on:?}"
    );
}

#[test]
fn settings_are_read_and_what_cannot_be_is_said_or_refused() {
    let read = |json: &str| PrintSettings::from_settings(&Settings::from_json(json).unwrap());
    // The reference's defaults: no scarf, but one on compensated circles.
    let plain = read("{}").unwrap();
    assert_eq!(plain.scarf, Scarf::default());
    assert!(plain.scarf.on_circles && plain.scarf.kind == ScarfSeam::None);
    let set = read(
        r#"{"seam_slope_type": "all", "seam_slope_conditional": "1", "scarf_angle_threshold": "140",
            "scarf_overhang_threshold": "25%", "scarf_joint_speed": "35", "scarf_joint_flow_ratio": "0.95",
            "seam_slope_start_height": "10%", "seam_slope_entire_loop": "1", "seam_slope_min_length": "12",
            "seam_slope_steps": "7", "seam_slope_inner_walls": "1", "apply_scarf_seam_on_circles": "0"}"#,
    )
    .unwrap();
    assert_eq!(
        set.scarf,
        Scarf {
            kind: ScarfSeam::All,
            conditional: true,
            angle_threshold: 140.0,
            overhang_threshold: 0.25,
            joint_speed: FloatOrPercent::Value(35.0),
            joint_flow_ratio: 0.95,
            start_height: FloatOrPercent::Percent(10.0),
            entire_loop: true,
            min_length: 12.0,
            steps: 7,
            inner_walls: true,
            on_circles: false,
        }
    );
    assert_eq!(
        read(r#"{"seam_slope_type": "external", "scarf_joint_speed": "80%"}"#)
            .unwrap()
            .scarf,
        Scarf {
            kind: ScarfSeam::External,
            joint_speed: FloatOrPercent::Percent(80.0),
            ..Scarf::default()
        }
    );
    // Every one of them is followed: nothing is said of any.
    for followed in [&plain, &set] {
        assert!(
            (followed.unsupported.iter())
                .all(|note| !note.contains("scarf") && !note.contains("slope")),
            "{:?}",
            followed.unsupported
        );
    }
    // Nor of the scarf on compensated circles, which is made now.
    let circles = read(r#"{"enable_circle_compensation": "1"}"#).unwrap();
    assert!(
        circles
            .unsupported
            .iter()
            .all(|note| !note.contains("circle"))
    );
    // Never in spiral vase mode.
    let vase = read(r#"{"seam_slope_type": "all", "spiral_mode": "1"}"#).unwrap();
    assert!(vase.scarf.kind == ScarfSeam::None && !vase.scarf.on_circles);
    // A joint speed of nothing is the wall's own, and that is said
    // where there are scarf seams to lay.
    let stopped = read(r#"{"seam_slope_type": "all", "scarf_joint_speed": "0"}"#).unwrap();
    assert_eq!(stopped.scarf.joint_speed, FloatOrPercent::Percent(100.0));
    assert_eq!(
        (stopped.unsupported.iter())
            .filter(|note| note.contains("scarf"))
            .collect::<Vec<_>>(),
        ["scarf_joint_speed \"0\": scarf joints are laid at their walls' speed"]
    );
    assert!(
        (read(r#"{"scarf_joint_speed": "0"}"#)
            .unwrap()
            .unsupported
            .iter())
        .all(|note| !note.contains("scarf"))
    );
    // What makes no sense stops the slice.
    for (key, value) in [
        ("seam_slope_type", "sometimes"),
        ("seam_slope_steps", "0"),
        ("seam_slope_min_length", "-1"),
        ("seam_slope_start_height", "-0.1"),
        ("scarf_joint_flow_ratio", "0"),
        ("scarf_joint_flow_ratio", "3"),
        ("scarf_joint_speed", "-5"),
        ("scarf_angle_threshold", "200"),
        ("scarf_overhang_threshold", "some"),
        ("seam_slope_conditional", "maybe"),
    ] {
        let refused = read(&format!(r#"{{"{key}": "{value}"}}"#));
        assert!(
            matches!(&refused, Err(ConfigError::BadValue { key: named, .. }) if named == key),
            "{key} {value}: {refused:?}"
        );
    }
}
