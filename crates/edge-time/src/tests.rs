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
