//! The Calibration menu's prints, made and sliced through the whole
//! pipeline with a printer's presets, and their G-code read back: the
//! commands each writes, their order and their values at chosen layers,
//! against the reference's code path (GCode.cpp `process_layer`, calib.cpp)
//! worked by hand. The reference's command line cannot make these prints
//! (its CLI has no calibration options), so this is the check.

use std::path::PathBuf;

use edge_config::{Settings, Value};
use edge_engine::calib::{CalibMode, CalibParams, CalibProject};
use edge_engine::{Bed, Chosen, Library, Overrides, Unwatched};

fn reference(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// The presets of a vendor's printer, its usual process and filament.
fn presets(vendor: &str, machine: &str) -> Chosen {
    let library = Library::new(reference("resources/profiles"));
    let shelf = library.shelf(vendor).unwrap();
    let (process, filament) = shelf.usual_for(machine);
    let filament = filament.unwrap();
    shelf
        .choose(Some(machine), process.as_deref(), &[filament.as_str()])
        .unwrap()
}

fn bed(settings: &Settings) -> Bed {
    let print = edge_gcode::PrintSettings::from_settings(settings).unwrap();
    Bed::of(print.bed.unwrap())
}

/// What the calibration sets, laid over the presets as the desktop app
/// lays what is set by hand over them.
fn overrides(made: &CalibProject) -> Overrides {
    let mut settings = Settings::default();
    for (key, value) in made.process.iter().chain(&made.printer) {
        settings.insert(key.clone(), Value::One(value.clone()));
    }
    let mut filament = Settings::default();
    for (key, value) in &made.filament {
        filament.insert(key.clone(), Value::One(value.clone()));
    }
    Overrides {
        settings,
        filaments: vec![filament],
        calibration: Some(made.params.clone()),
        ..Overrides::default()
    }
}

/// Makes the calibration for the printer and slices it: its G-code.
fn sliced_gcode(chosen: &Chosen, params: &CalibParams) -> (CalibProject, String) {
    let settings = edge_engine::settled(chosen, &Overrides::default(), None).unwrap();
    let bed = bed(&settings);
    let calib = reference("resources/calib");
    let made = match params.mode {
        CalibMode::TemperatureTower => edge_engine::calib::temperature(params, &calib, Some(&bed)),
        _ => edge_engine::calib::pressure_advance(params, &settings, &calib, Some(&bed)),
    }
    .unwrap();
    let loaded = made
        .model
        .place(made.model.plates().first().map(|plate| plate.id()))
        .unwrap();
    let mut set = overrides(&made);
    set.calibration = Some(made.params.on_plate(
        &made.model,
        made.model.plates().first().map(|plate| plate.id()),
    ));
    let job = edge_engine::prepare(loaded, chosen, &set).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let text = sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text;
    (made, String::from_utf8(text).unwrap())
}

/// Each layer's text, from its `;LAYER_CHANGE` on.
fn layers(gcode: &str) -> Vec<&str> {
    gcode.split(";LAYER_CHANGE\n").skip(1).collect()
}

#[test]
fn the_pa_tower_sets_its_pressure_advance_at_every_layer_change() {
    let chosen = presets("Snapmaker", "Snapmaker U1 (0.4 nozzle)");
    let params = CalibParams::new(CalibMode::PressureAdvanceTower, 0.0, 0.1, 0.002);
    let (made, gcode) = sliced_gcode(&chosen, &params);
    let layers = layers(&gcode);
    // 51 mm of tower: a 0.2 mm first layer and 0.2 mm layers above.
    let count = layers.len();
    assert_eq!(count, 255, "layers");
    // GCode.cpp `process_layer`: after the layer change G-code and the
    // fan marker, before anything of the layer, the PA of its band: band
    // floor(index / (count - 1) * 51), held to 50; a U1 is Klipper.
    let expected = |index: usize| -> String {
        let value = edge_gcode::calib::stepped_layer_value(index, count, 0.0, 0.1, 0.002);
        format!(
            "SET_PRESSURE_ADVANCE ADVANCE={}; Override pressure advance value",
            edge_gcode::calib::general(f64::from(value), 4)
        )
    };
    for (index, text) in layers.iter().enumerate() {
        let after = text
            .split_once(";_SET_FAN_SPEED_CHANGING_LAYER\n")
            .expect("the fan marker")
            .1;
        let line = after.lines().next().unwrap();
        assert_eq!(line, expected(index), "layer {index}");
    }
    let at = |index: usize| {
        layers[index]
            .lines()
            .find(|line| line.starts_with("SET_PRESSURE_ADVANCE"))
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        at(0),
        "SET_PRESSURE_ADVANCE ADVANCE=0; Override pressure advance value"
    );
    // 5 / 254 * 51 = 1.004: the second band.
    assert_eq!(
        at(5),
        "SET_PRESSURE_ADVANCE ADVANCE=0.002; Override pressure advance value"
    );
    // 127 / 254 * 51 = 25.5: band 25, 0.05.
    assert_eq!(
        at(127),
        "SET_PRESSURE_ADVANCE ADVANCE=0.05; Override pressure advance value"
    );
    assert_eq!(
        at(254),
        "SET_PRESSURE_ADVANCE ADVANCE=0.1; Override pressure advance value"
    );
    // The walls print at the test's speed, which the object carries.
    let speed: f64 = made.model.objects()[0]
        .settings()
        .iter()
        .find(|(key, _)| key == "outer_wall_speed")
        .unwrap()
        .1
        .parse()
        .unwrap();
    assert!(speed >= 100.0, "{speed}");
    assert!(
        gcode.contains(&format!("F{}", speed * 60.0)),
        "walls at {speed} mm/s"
    );
    // Two walls, no top, bottom or fill.
    assert!(!gcode.contains(";TYPE:Sparse infill"));
    assert!(!gcode.contains(";TYPE:Top surface"));
}

#[test]
fn a_marlin_printer_is_told_with_m900() {
    let chosen = presets("Creality", "Creality Ender-3 V3 SE 0.4 nozzle");
    let params = CalibParams::new(CalibMode::PressureAdvanceTower, 0.0, 0.02, 0.002);
    let (_, gcode) = sliced_gcode(&chosen, &params);
    let layers = layers(&gcode);
    assert!(layers[0].contains("\nM900 K0; Override pressure advance value\n"));
    assert!(
        layers
            .last()
            .unwrap()
            .contains("\nM900 K0.02; Override pressure advance value\n")
    );
}

#[test]
fn the_temperature_tower_cools_its_nozzle_block_by_block() {
    let chosen = presets("Snapmaker", "Snapmaker U1 (0.4 nozzle)");
    // Two blocks: 230 and 225, 20 mm.
    let params = CalibParams::new(CalibMode::TemperatureTower, 230.0, 225.0, 5.0);
    let (_, gcode) = sliced_gcode(&chosen, &params);
    // The filament's own temperature is the start's, from the first layer.
    assert!(
        gcode.contains("M109 S230") || gcode.contains("S230"),
        "start"
    );
    let layers = layers(&gcode);
    // GCode.cpp `process_layer`: at every layer, after the fan marker,
    // 5 degrees less for each whole 10.001 mm below the layer's top.
    for text in &layers {
        let z: f64 = text
            .lines()
            .find_map(|line| line.strip_prefix(";Z:"))
            .unwrap()
            .parse()
            .unwrap();
        let after = text
            .split_once(";_SET_FAN_SPEED_CHANGING_LAYER\n")
            .unwrap()
            .1;
        // The U1's first layer is 0.25 mm: the last layer's top is 20.05,
        // over 2 × 10.001, so it prints at 220 as in the reference.
        let expected = 230 - 5 * (z / 10.001) as u32;
        assert_eq!(
            after.lines().next().unwrap(),
            format!("M104 S{expected} ; set nozzle temperature"),
            "at {z}"
        );
    }
    let top: f64 = layers
        .last()
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(";Z:"))
        .unwrap()
        .parse()
        .unwrap();
    assert!((top - 20.05).abs() < 1e-6, "{top}");
    assert!(
        layers
            .last()
            .unwrap()
            .contains("M104 S220 ; set nozzle temperature")
    );
}

#[test]
fn the_pa_line_is_drawn_in_place_of_the_model() {
    let chosen = presets("Snapmaker", "Snapmaker U1 (0.4 nozzle)");
    let mut params = CalibParams::new(CalibMode::PressureAdvanceLine, 0.0, 0.1, 0.002);
    params.print_numbers = true;
    let (_, gcode) = sliced_gcode(&chosen, &params);
    // No layer of the model: the test's two (its numbers on the second).
    assert_eq!(
        gcode.matches(";LAYER_CHANGE").count(),
        1,
        "the numbers' layer"
    );
    // calib.cpp `print_pa_lines`: 0 for the anchor, then each line's own,
    // 0 again: 51 lines on a 270 mm bed, (270 - 10) / 3.5 = 74 at most.
    let advances: Vec<&str> = gcode
        .lines()
        .filter(|line| line.starts_with("SET_PRESSURE_ADVANCE"))
        .collect();
    assert_eq!(advances.len(), 1 + 51 + 1 + 1, "{advances:?}");
    assert_eq!(
        advances[1],
        "SET_PRESSURE_ADVANCE ADVANCE=0; Override pressure advance value"
    );
    assert_eq!(
        advances[2],
        "SET_PRESSURE_ADVANCE ADVANCE=0; Override pressure advance value"
    );
    assert_eq!(
        advances[3],
        "SET_PRESSURE_ADVANCE ADVANCE=0.002; Override pressure advance value"
    );
    assert_eq!(
        advances[52],
        "SET_PRESSURE_ADVANCE ADVANCE=0.1; Override pressure advance value"
    );
    // The end G-code follows.
    assert!(gcode.contains("; filament used [mm]"));
}

#[test]
fn the_pa_pattern_draws_round_its_handle_on_the_first_four_layers() {
    let chosen = presets("Snapmaker", "Snapmaker U1 (0.4 nozzle)");
    let params = CalibParams::new(CalibMode::PressureAdvancePattern, 0.0, 0.08, 0.005);
    let (made, gcode) = sliced_gcode(&chosen, &params);
    let names: Vec<&str> = made
        .model
        .objects()
        .iter()
        .map(|object| object.name())
        .collect();
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with("pa_pattern_"), "{names:?}");
    assert!(
        made.notes
            .iter()
            .any(|(said, _)| said.starts_with("No speeds provided"))
    );
    let layers = layers(&gcode);
    assert_eq!(layers.len(), 4, "the handle's four layers");
    for (index, text) in layers.iter().enumerate() {
        assert!(
            text.contains(";CUSTOM_GCODE\n; start pressure advance pattern for layer\n"),
            "layer {index}"
        );
        for j in 0..17 {
            let value = edge_gcode::calib::general(f64::from(j) * 0.005, 4);
            assert!(
                text.contains(&format!("ADVANCE={value}; Override")),
                "{index} {j}"
            );
        }
    }
}
