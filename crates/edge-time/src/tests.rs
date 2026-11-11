use super::*;

fn klipper(acceleration: f64, top_speed: f64) -> Limits {
    Limits {
        flavor: Flavor::Klipper,
        max_speed: [top_speed, top_speed, 20.0, 60.0],
        max_acceleration: [20_000.0, 20_000.0, 500.0, 5_000.0],
        max_extruding: acceleration,
        max_retracting: 5_000.0,
        max_travel: acceleration,
        jerk: [5.0, 5.0, 3.0, 2.5],
        ..Limits::default()
    }
}

fn close(found: f64, expected: f64) -> bool {
    (found - expected).abs() < 1e-6
}

#[test]
fn a_long_move_speeds_up_cruises_and_slows_down() {
    // 100 mm at 100 mm/s with 1000 mm/s²: 5 mm and 0.1 s to speed up, the
    // same to stop, 90 mm at speed between.
    let limits = klipper(1000.0, 500.0);
    for model in [Model::Reference, Model::Klipper] {
        let estimate = estimate("G1 X100 F6000\n", &limits, model);
        assert!(
            close(estimate.seconds, 1.1),
            "{model:?}: {}",
            estimate.seconds
        );
        assert_eq!(estimate.by_line.len(), 1);
    }
    // At the speed alone it would be a second.
}

#[test]
fn klipper_holds_a_short_moves_peak_to_the_cruise_ratio() {
    // 10 mm, too short to reach 200 mm/s at 1000 mm/s². Flat out it would
    // peak at 100 mm/s in the middle: 0.2 s. With half the move to be
    // spent cruising, Klipper peaks at 70.7 mm/s: 2.5 mm up, 5 mm along,
    // 2.5 mm down.
    let limits = klipper(1000.0, 500.0);
    let ratio = |ratio: &str| {
        let gcode =
            format!("SET_VELOCITY_LIMIT ACCEL=1000 MINIMUM_CRUISE_RATIO={ratio}\nG1 X10 F12000\n");
        estimate(&gcode, &limits, Model::Klipper).seconds
    };
    assert!(close(ratio("0"), 0.2), "{}", ratio("0"));
    let peak = 5000f64.sqrt();
    let expected = 2.0 * 2.5 / (peak / 2.0) + 5.0 / peak;
    assert!(
        close(ratio("0.5"), expected),
        "{} against {expected}",
        ratio("0.5")
    );
    // A half is also what a printer has when the file says nothing.
    assert!(close(
        estimate("M204 S1000\nG1 X10 F12000\n", &limits, Model::Klipper).seconds,
        expected
    ));
    // The older way of saying it comes to the same.
    let legacy = "SET_VELOCITY_LIMIT ACCEL=1000 ACCEL_TO_DECEL=500\nG1 X10 F12000\n";
    assert!(close(
        estimate(legacy, &limits, Model::Klipper).seconds,
        expected
    ));
    // The reference's estimator does not know of it.
    let reference = estimate(
        "SET_VELOCITY_LIMIT ACCEL=1000 MINIMUM_CRUISE_RATIO=0.5\nG1 X10 F12000\n",
        &limits,
        Model::Reference,
    );
    assert!(close(reference.seconds, 0.2), "{}", reference.seconds);
}

#[test]
fn a_square_corner_is_taken_at_the_square_corner_velocity() {
    // Two long moves at right angles: both planners pass the corner at
    // 5 mm/s, so the time is that of a move from rest to 5 mm/s and one
    // from 5 mm/s to rest.
    let limits = klipper(1000.0, 500.0);
    let gcode = "SET_VELOCITY_LIMIT ACCEL=1000 MINIMUM_CRUISE_RATIO=0\nG1 X100 F6000\nG1 Y100\n";
    // One leg: up to 100 mm/s (0.1 s, 5 mm), down to 5 mm/s (0.095 s,
    // 4.9875 mm), the rest at speed.
    let leg = 0.1 + 0.095 + (100.0 - 5.0 - 4.9875) / 100.0;
    for model in [Model::Reference, Model::Klipper] {
        let estimate = estimate(gcode, &limits, model);
        assert!(
            close(estimate.seconds, 2.0 * leg),
            "{model:?}: {}",
            estimate.seconds
        );
        assert!(close(estimate.by_line[1], leg) && close(estimate.by_line[2], leg));
    }
}

#[test]
fn waits_and_tool_changes_are_counted_where_they_are_written() {
    let limits = Limits {
        tool_change_time: 5.0,
        ..klipper(1000.0, 500.0)
    };
    let gcode = "G1 X100 F6000\nG4 P1500\nG1 X0\nT0\nT1\nG1 X100\n";
    for model in [Model::Reference, Model::Klipper] {
        let estimate = estimate(gcode, &limits, model);
        // Each move stops for what follows it. The first tool is in
        // hand already; the second takes its time.
        assert!(
            close(estimate.seconds, 3.0 * 1.1 + 1.5 + 5.0),
            "{model:?}: {}",
            estimate.seconds
        );
        assert!(close(estimate.by_line[1], 1.5));
        assert_eq!(estimate.by_line[3], 0.0);
        assert!(close(estimate.by_line[4], 5.0));
        let elapsed = estimate.elapsed();
        assert!(close(elapsed[5], estimate.seconds) && close(elapsed[0], 1.1));
    }
}

#[test]
fn relative_moves_arcs_and_comments_are_read() {
    let limits = klipper(1000.0, 500.0);
    // The same 100 mm, said as two relative moves that run on.
    let relative = "G91 ; relative\nG1 X50 F6000 ; half\nG1 X50\n; nothing\n";
    for model in [Model::Reference, Model::Klipper] {
        let estimate = estimate(relative, &limits, model);
        assert!(
            close(estimate.seconds, 1.1),
            "{model:?}: {}",
            estimate.seconds
        );
        assert_eq!(estimate.by_line.len(), 4);
    }
    // A quarter circle of 10 mm radius is 15.7 mm of path: more than the
    // 14.1 mm straight across, in more time.
    let arc = estimate(
        "G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0\n",
        &limits,
        Model::Klipper,
    );
    let straight = estimate("G1 X10 Y0 F600\nG1 X0 Y10\n", &limits, Model::Klipper);
    assert!(arc.seconds > straight.seconds);
    assert!(
        (arc.by_line[1] - std::f64::consts::FRAC_PI_2).abs() < 0.05,
        "{}",
        arc.by_line[1]
    );
}

#[test]
fn a_file_with_nothing_to_time_takes_no_time() {
    let estimate = estimate(
        "; only a comment\nM104 S200\n",
        &Limits::default(),
        Model::Reference,
    );
    assert_eq!(estimate.seconds, 0.0);
    assert_eq!(estimate.by_line, [0.0, 0.0]);
    assert_eq!(
        super::estimate("", &Limits::default(), Model::Klipper).seconds,
        0.0
    );
}

#[test]
fn limits_are_read_from_a_printer_profile() {
    let settings = edge_config::Settings::from_json(
        r#"{"gcode_flavor": "klipper", "machine_max_speed_x": ["500", "200"],
            "machine_max_acceleration_extruding": ["20000", "20000"],
            "machine_max_acceleration_z": ["500", "200"], "machine_max_jerk_x": ["9", "9"],
            "machine_tool_change_time": "5"}"#,
    )
    .unwrap();
    let limits = Limits::from_settings(&settings).unwrap();
    assert_eq!(limits.flavor, Flavor::Klipper);
    assert_eq!(limits.own_model(), Model::Klipper);
    assert_eq!(
        (limits.max_speed[0], limits.max_extruding),
        (500.0, 20_000.0)
    );
    assert_eq!((limits.max_acceleration[2], limits.jerk[0]), (500.0, 9.0));
    assert_eq!(limits.tool_change_time, 5.0);
    assert_eq!(
        Limits::from_settings(&edge_config::Settings::default())
            .unwrap()
            .own_model(),
        Model::Reference
    );
}

#[test]
fn ironing_is_timed_as_the_moves_it_is() {
    // Ironing is written as ordinary extruding moves under a name of
    // its own, slow and with next to nothing fed: 100 mm at 30 mm/s
    // with 1000 mm/s² is 0.45 mm and 0.03 s to speed up, the same to
    // stop, and 99.1 mm at speed between.
    let limits = klipper(1000.0, 500.0);
    let ironing = ";TYPE:Ironing\n;WIDTH:0.402\nG1 X100 E0.12 F1800\n";
    let expected = 0.06 + 99.1 / 30.0;
    for model in [Model::Reference, Model::Klipper] {
        let found = estimate(ironing, &limits, model).seconds;
        assert!(close(found, expected), "{model:?}: {found}");
        // The name changes nothing: the same move under another takes
        // as long.
        let top = ironing.replace("Ironing", "Top surface");
        assert!(close(estimate(&top, &limits, model).seconds, found));
    }
    // A sweep of short lines and the turns between them is the sum of
    // its moves, each begun and ended at the speed the corner allows.
    let sweep = ";TYPE:Ironing\nG1 X10 E0.01 F1800\nG1 Y0.15 E0.0002\nG1 X0 E0.01\n";
    for model in [Model::Reference, Model::Klipper] {
        let found = estimate(sweep, &limits, model);
        assert_eq!(found.by_line.len(), 4);
        assert!(found.seconds > 20.15 / 30.0 && found.seconds < 20.15 / 30.0 + 0.2);
    }
}

/// A planner that only keeps what it is told to do: each move as the
/// line it came from, how far it goes (x, y, z, e) and how fast.
#[derive(Default)]
struct Told {
    moves: Vec<(usize, [f64; 4], f64)>,
}

impl Planner for &mut Told {
    fn go(&mut self, line: usize, delta: [f64; 4], feed: f64) {
        self.moves.push((line, delta, feed));
    }
    fn rest(&mut self, _line: usize, _extra: f64) {}
    fn set(&mut self, _what: reader::Set) {}
    fn finish(self, lines: usize) -> Vec<f64> {
        vec![0.0; lines]
    }
}

/// The moves a file is read as, for a firmware and a planner.
fn told(gcode: &str, flavor: Flavor, model: Model) -> Vec<(usize, [f64; 4], f64)> {
    let limits = Limits {
        flavor,
        ..Limits::default()
    };
    let mut told = Told::default();
    reader::read(gcode, &limits, model, &mut told);
    told.moves
}

/// Where a run of moves ends up, and how far it went across the bed.
fn summed(moves: &[(usize, [f64; 4], f64)]) -> ([f64; 4], f64) {
    let mut at = [0.0; 4];
    let mut length = 0.0;
    for (_, delta, _) in moves {
        for axis in 0..4 {
            at[axis] += delta[axis];
        }
        length += delta[0].hypot(delta[1]);
    }
    (at, length)
}

#[test]
fn the_reference_cuts_an_arc_as_the_files_firmware_would() {
    // A quarter circle of 10 mm radius, counter-clockwise from (10, 0)
    // to (0, 10), feeding 1 mm of filament at 10 mm/s.
    let quarter = "G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0 E1\n";
    let arc = 10.0 * std::f64::consts::FRAC_PI_2;
    // Chords within 0.0125 mm of the arc: each spans twice the angle
    // whose cosine is 1 - 0.0125/10, a tenth of a radian, so 16 of them.
    for flavor in [Flavor::Klipper, Flavor::Other] {
        let moves = told(quarter, flavor, Model::Reference);
        let cut = &moves[1..];
        assert_eq!(cut.len(), 16, "{flavor:?}");
        assert!(
            cut.iter()
                .all(|(line, _, feed)| *line == 1 && *feed == 10.0)
        );
        let (end, length) = summed(cut);
        assert!(close(end[0], -10.0) && close(end[1], 10.0) && close(end[3], 1.0));
        assert!(length < arc && arc - length < 0.01, "{length}");
        // Every chord is as long as the next, and feeds as much.
        for (_, delta, _) in cut {
            assert!((delta[0].hypot(delta[1]) - length / 16.0).abs() < 1e-3);
            assert!((delta[3] - 1.0 / 16.0).abs() < 1e-9);
        }
    }
    // Marlin 2 cuts by length: no chord further than 0.02 mm from the
    // arc, 50 moves a second at least, so 0.2 mm each at this speed.
    let moves = told(quarter, Flavor::Marlin2, Model::Reference);
    let cut = &moves[1..];
    assert_eq!(cut.len(), 79);
    let (end, length) = summed(cut);
    assert!(close(end[0], -10.0) && close(end[1], 10.0));
    assert!((end[3] - 1.0).abs() < 1e-6);
    assert!(length < arc && arc - length < 0.001, "{length}");
    // Faster, the moves are longer: 1.265 mm, the most that stays
    // within 0.02 mm of the arc.
    let fast = told(
        "G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0 F12000\n",
        Flavor::Marlin2,
        Model::Reference,
    );
    assert_eq!(fast.len() - 1, 13);
    assert!(fast[1..].iter().all(|(_, _, feed)| *feed == 200.0));

    // Clockwise the long way round between the same two points.
    let moves = told(
        "G1 X10 Y0 F600\nG2 X0 Y10 I-10 J0\n",
        Flavor::Klipper,
        Model::Reference,
    );
    let (end, length) = summed(&moves[1..]);
    assert!(close(end[0], -10.0) && close(end[1], 10.0));
    assert!((length - 3.0 * arc).abs() < 0.05, "{length}");
}

#[test]
fn a_spiral_lift_is_read_as_one_rising_turn() {
    // As both slicers write it: no X or Y, so it ends where it starts,
    // 0.6 mm up, round a centre 1.217 mm away.
    let lift = "G1 X50 Y50 Z0.2 F6000\nG17\nG3 Z0.8 I-0.089 J1.214 P1 F15343\n";
    let radius = 0.089f64.hypot(1.214);
    let around = std::f64::consts::TAU * radius;
    for (model, flavor) in [
        (Model::Reference, Flavor::Klipper),
        (Model::Reference, Flavor::Marlin2),
        (Model::Klipper, Flavor::Klipper),
    ] {
        let moves = told(lift, flavor, model);
        let cut = &moves[1..];
        assert!(cut.len() > 5, "{model:?}: {} moves", cut.len());
        let (end, length) = summed(cut);
        assert!(end[0].abs() < 1e-5 && end[1].abs() < 1e-5, "{model:?}");
        assert!((end[2] - 0.6).abs() < 1e-6, "{model:?}: {end:?}");
        assert!(
            length < around && around - length < 0.3,
            "{model:?}: {length}"
        );
        // It climbs evenly on the way.
        let rise = 0.6 / cut.len() as f64;
        assert!(
            cut.iter()
                .all(|(_, delta, _)| (delta[2] - rise).abs() < 1e-6)
        );
    }
    // Klipper cuts it into moves a millimetre long, as near as a whole
    // number of them comes: seven for 7.67 mm.
    assert_eq!(told(lift, Flavor::Klipper, Model::Klipper).len() - 1, 7);
}

#[test]
fn klipper_cuts_an_arc_into_millimetre_moves() {
    let quarter = "G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0 E1\n";
    let moves = told(quarter, Flavor::Klipper, Model::Klipper);
    // 15.7 mm of arc: 15 moves.
    let cut = &moves[1..];
    assert_eq!(cut.len(), 15);
    let (end, length) = summed(cut);
    assert!(close(end[0], -10.0) && close(end[1], 10.0) && close(end[3], 1.0));
    let arc = 10.0 * std::f64::consts::FRAC_PI_2;
    assert!(length < arc && arc - length < 0.01, "{length}");
    // An arc shorter than a millimetre is one straight move.
    let short = told(
        "G1 X10 Y0 F600\nG3 X9.95 Y0.998 I-10 J0\n",
        Flavor::Klipper,
        Model::Klipper,
    );
    assert_eq!(short.len() - 1, 1);
    // A speed factor applies to an arc as to any move.
    let slowed = told(
        "M220 S50\nG1 X10 Y0 F600\nG3 X0 Y10 I-10 J0\n",
        Flavor::Klipper,
        Model::Klipper,
    );
    assert!(slowed.iter().all(|(_, _, feed)| *feed == 5.0));
}

#[test]
fn an_arc_with_no_centre_is_not_run() {
    // Neither planner moves for an arc that does not say where its
    // centre is, nor takes its feed rate; the next move goes from where
    // the nozzle still is, at the speed set before.
    let gcode = "G1 X10 Y0 F600\nG3 X0 Y10 F6000\nG1 X20 Y0\n";
    for model in [Model::Reference, Model::Klipper] {
        let moves = told(gcode, Flavor::Klipper, model);
        assert_eq!(moves.len(), 2, "{model:?}");
        assert_eq!(moves[1], (2, [10.0, 0.0, 0.0, 0.0], 10.0));
    }
    // Given as a radius, the reference runs it; Klipper refuses.
    let by_radius = "G1 X10 Y0 F600\nG3 X0 Y10 R10\n";
    let moves = told(by_radius, Flavor::Klipper, Model::Reference);
    let (end, length) = summed(&moves[1..]);
    assert!((end[0] + 10.0).abs() < 1e-5 && (end[1] - 10.0).abs() < 1e-5);
    assert!((length - 10.0 * std::f64::consts::FRAC_PI_2).abs() < 0.01);
    assert_eq!(told(by_radius, Flavor::Klipper, Model::Klipper).len(), 1);
    // A negative radius is the long way round.
    let long = told(
        "G1 X10 Y0 F600\nG3 X0 Y10 R-10\n",
        Flavor::Klipper,
        Model::Reference,
    );
    let (_, length) = summed(&long[1..]);
    assert!((length - 30.0 * std::f64::consts::FRAC_PI_2).abs() < 0.05);
    // Klipper has no arcs while moves are relative; the reference does.
    let relative = "G1 X10 Y0 F600\nG91\nG3 X-10 Y10 I-10 J0\n";
    assert_eq!(told(relative, Flavor::Klipper, Model::Klipper).len(), 1);
    assert_eq!(told(relative, Flavor::Klipper, Model::Reference).len(), 17);
    // An arc of under a micron is nothing to the reference.
    let speck = "G1 X10 Y0 F600\nG3 X10.0002 Y0.0002 I0.0001 J0.0001\n";
    assert_eq!(told(speck, Flavor::Klipper, Model::Reference).len(), 1);
}

#[test]
fn an_arc_takes_the_time_of_its_length() {
    // A quarter circle of 10 mm radius at 10 mm/s: 15.7 mm, a little
    // over a second and a half however it is cut.
    let limits = klipper(1000.0, 500.0);
    let quarter = std::f64::consts::FRAC_PI_2;
    for model in [Model::Reference, Model::Klipper] {
        let arc = estimate("G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0\n", &limits, model);
        assert!(
            (arc.by_line[1] - quarter).abs() < 0.02,
            "{model:?}: {}",
            arc.by_line[1]
        );
        assert_eq!(arc.by_line.len(), 2);
    }
    let marlin = Limits {
        flavor: Flavor::Marlin2,
        ..limits
    };
    let arc = estimate(
        "G1 X10 Y0 F600\nG3 X0 Y10 I-10 J0\n",
        &marlin,
        Model::Reference,
    );
    assert!(
        (arc.by_line[1] - quarter).abs() < 0.02,
        "{}",
        arc.by_line[1]
    );
}
