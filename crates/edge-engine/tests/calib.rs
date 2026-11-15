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
        calibration: made.params.clone(),
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
        CalibMode::RetractionTower => {
            edge_engine::calib::retraction(params, &settings, &calib, Some(&bed))
        }
        _ => edge_engine::calib::pressure_advance(params, &settings, &calib, Some(&bed)),
    }
    .unwrap();
    let gcode = slice_made(chosen, &made).0;
    (made, gcode)
}

/// Slices a calibration's project as made: its G-code and the slice's
/// notes.
fn slice_made(chosen: &Chosen, made: &CalibProject) -> (String, Vec<String>) {
    let loaded = made
        .model
        .place(made.model.plates().first().map(|plate| plate.id()))
        .unwrap();
    let mut set = overrides(made);
    set.calibration = made.params.as_ref().map(|params| {
        params.on_plate(
            &made.model,
            made.model.plates().first().map(|plate| plate.id()),
        )
    });
    let job = edge_engine::prepare(loaded, chosen, &set).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let notes = sliced.gaps();
    let text = sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text;
    (String::from_utf8(text).unwrap(), notes)
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

/// Each object's moves of a kind over a mm long, by the object's name:
/// the filament each feeds per mm, from the G-code's object markers
/// (`EXCLUDE_OBJECT_START NAME=…`) and `;TYPE:` lines.
fn fed_per_mm(gcode: &str, kind: &str) -> std::collections::BTreeMap<String, Vec<f64>> {
    let mut out = std::collections::BTreeMap::new();
    let (mut object, mut typed) = (String::new(), false);
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    for line in gcode.lines() {
        if let Some(name) = line.strip_prefix("EXCLUDE_OBJECT_START NAME=") {
            object = name.split("_id_").next().unwrap_or(name).to_owned();
        } else if line.starts_with("EXCLUDE_OBJECT_END") {
            object.clear();
        } else if let Some(role) = line.strip_prefix(";TYPE:") {
            typed = role == kind;
        } else if line.starts_with("G1 ") || line.starts_with("G0 ") {
            let word = |letter: char| {
                line.split_whitespace()
                    .find_map(|w| w.strip_prefix(letter)?.parse::<f64>().ok())
            };
            let (nx, ny) = (word('X').unwrap_or(x), word('Y').unwrap_or(y));
            if let Some(e) = word('E')
                && e > 0.0
                && typed
                && !object.is_empty()
            {
                let length = (nx - x).hypot(ny - y);
                if length > 1.0 {
                    out.entry(object.clone())
                        .or_insert_with(Vec::new)
                        .push(e / length);
                }
            }
            (x, y) = (nx, ny);
        }
    }
    out
}

#[test]
fn the_flow_rate_tiles_each_print_their_own_flow() {
    use edge_engine::calib::FlowTest;
    let chosen = presets("Snapmaker", "Snapmaker U1 (0.4 nozzle)");
    let settings = edge_engine::settled(&chosen, &Overrides::default(), None).unwrap();
    let bed = bed(&settings);
    let made = edge_engine::calib::flow_rate(
        FlowTest::Pass1,
        &settings,
        &reference("resources/calib"),
        Some(&bed),
    )
    .unwrap();
    assert_eq!(made.name, "Flowrate Test - Pass1");
    assert!(made.params.is_none());
    // Nine tiles, in the file's order, each its own flow ratio:
    // 1 + modifier / 100 (Plater.cpp, pass 1 and 2).
    let names: Vec<&str> = made.model.objects().iter().map(|o| o.name()).collect();
    assert_eq!(
        names,
        [
            "flowrate_0",
            "flowrate_10",
            "flowrate_15",
            "flowrate_20",
            "flowrate_5",
            "flowrate_m10",
            "flowrate_m15",
            "flowrate_m20",
            "flowrate_m5"
        ]
    );
    let own = |object: usize, key: &str| -> String {
        made.model.objects()[object]
            .settings()
            .iter()
            .find(|(own, _)| own == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{key} on {object}"))
    };
    assert_eq!(own(0, "print_flow_ratio"), "1");
    assert_eq!(own(1, "print_flow_ratio"), "1.10000002384186");
    assert_eq!(own(5, "print_flow_ratio"), "0.899999976158142");
    assert_eq!(own(0, "top_surface_pattern"), "archimedeanchords");
    assert_eq!(own(0, "calib_flowrate_topinfill_special_order"), "1");
    // 1.2 nozzles in single precision, as the reference sets it.
    assert_eq!(own(0, "top_surface_line_width"), "0.48000002");
    // Over the process: what is the same on every tile and not followed
    // on an object, and what the reference sets there itself.
    let process: std::collections::BTreeMap<&str, &str> = made
        .process
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    for (key, value) in [
        ("layer_height", "0.2"),
        ("initial_layer_print_height", "0.25"),
        ("seam_slope_type", "none"),
        ("enable_extra_bridge_layer", "disabled"),
        ("internal_bridge_density", "100%"),
        ("spiral_mode", "0"),
        ("reduce_crossing_wall", "1"),
        ("max_volumetric_extrusion_rate_slope", "0"),
    ] {
        assert_eq!(process.get(key), Some(&value), "{key}");
    }
    assert!(!process.contains_key("print_flow_ratio"));
    assert!(!process.contains_key("calib_flowrate_topinfill_special_order"));
    // Ten layers: (0.25 + 9 × 0.2) / 2 = 1.025 of the 2 mm tile.
    let copy = made.model.objects()[0].copies()[0].id();
    let [low, high] = made.model.in_scene(Some(&bed), copy).unwrap().1.unwrap();
    assert!((high[2] - low[2] - 2.05).abs() < 1e-3, "{low:?} {high:?}");
    // The group in the middle of the bed.
    let (mut least, mut most) = ([f64::MAX; 2], [f64::MIN; 2]);
    for object in made.model.objects() {
        let [low, high] = made
            .model
            .in_scene(Some(&bed), object.copies()[0].id())
            .unwrap()
            .1
            .unwrap();
        for axis in 0..2 {
            least[axis] = least[axis].min(low[axis]);
            most[axis] = most[axis].max(high[axis]);
        }
    }
    for axis in 0..2 {
        let middle = (bed.min[axis] + bed.max[axis]) / 2.0;
        assert!(((least[axis] + most[axis]) / 2.0 - middle).abs() < 1e-6);
    }

    let (gcode, notes) = slice_made(&chosen, &made);
    if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
        std::fs::write(
            std::path::Path::new(&dir).join("flow_pass1_u1.gcode"),
            &gcode,
        )
        .unwrap();
    }
    // Nothing the tiles ask for is left undone.
    assert!(
        !notes
            .iter()
            .any(|note| note.starts_with("top_surface_pattern") || note.contains("set on")),
        "{notes:?}"
    );
    assert_eq!(gcode.matches(";LAYER_CHANGE").count(), 10);
    // The top surfaces of each tile are fed in proportion to its flow:
    // the same lines, at the same speed, its own ratio.
    let fed = fed_per_mm(&gcode, "Top surface");
    // The most usual value: the lines of the tile's top, not its gaps.
    let per_mm = |name: &str| {
        let mut values = fed[name].clone();
        values.sort_by(f64::total_cmp);
        values[values.len() / 2]
    };
    let base = per_mm("flowrate_0");
    for (name, ratio) in [
        ("flowrate_10", 1.1),
        ("flowrate_m20", 0.8),
        ("flowrate_5", 1.05),
    ] {
        let got = per_mm(name) / base;
        assert!((got - ratio).abs() < 0.002, "{name}: {got}");
    }
}

/// The tower's layers' tops and how far the filament is pushed back on
/// after each retraction on them (`G1 E<length>`): the whole retraction,
/// which a wipe splits into parts.
fn retractions_by_layer(gcode: &str) -> Vec<(f64, Vec<f64>)> {
    layers(gcode)
        .iter()
        .map(|text| {
            let z: f64 = text
                .lines()
                .find_map(|line| line.strip_prefix(";Z:"))
                .unwrap()
                .parse()
                .unwrap();
            let pulls = text
                .lines()
                .filter_map(|line| line.strip_prefix("G1 E"))
                .filter(|rest| !rest.starts_with('-'))
                .map(|rest| rest.split_whitespace().next().unwrap().parse().unwrap())
                .collect();
            (z, pulls)
        })
        .collect()
}

#[test]
fn the_retraction_tower_lengthens_its_retraction_every_mm() {
    for (vendor, machine) in [
        ("Snapmaker", "Snapmaker U1 (0.4 nozzle)"),
        ("Creality", "Creality Ender-3 V3 SE 0.4 nozzle"),
    ] {
        let chosen = presets(vendor, machine);
        // 0 to 1 mm by 0.1: cut at 1.4 + 10 = 11.4 mm.
        let params = CalibParams::new(CalibMode::RetractionTower, 0.0, 1.0, 0.1);
        let (made, gcode) = sliced_gcode(&chosen, &params);
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("retraction_{vendor}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(made.name, "Retraction test");
        let process: std::collections::BTreeMap<&str, &str> = made
            .process
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(process["initial_layer_print_height"], "0.2");
        assert_eq!(process["wipe_inward"], "0");
        // Not followed on an object: over the process (one object).
        assert_eq!(process["seam_position"], "aligned");
        assert_eq!(process["seam_slope_type"], "none");
        assert!(
            made.printer
                .contains(&("use_firmware_retraction".into(), "0".into()))
        );
        let by_layer = retractions_by_layer(&gcode);
        // 11.4 mm at 0.2: 57 layers.
        assert_eq!(by_layer.len(), 57, "{machine}");
        // The layers' tops as the reference adds them up (Slicing.cpp
        // `generate_object_layers`: `print_z += height`, the height kept
        // in single precision), which the length is worked from: the
        // seventh layer's top is a little over 1.4 there, so its length
        // is already 0.1.
        let mut tops = vec![0.2_f64];
        while tops.len() < by_layer.len() {
            tops.push(tops[tops.len() - 1] + f64::from(0.2_f32));
        }
        for (index, text) in layers(&gcode).iter().enumerate() {
            let z = tops[index];
            assert!((z - by_layer[index].0).abs() < 1e-6);
            let after = text
                .split_once(";_SET_FAN_SPEED_CHANGING_LAYER\n")
                .unwrap()
                .1;
            // GCode.cpp: written after the fan marker, `%g` of each.
            let length = edge_gcode::calib::tower_retraction(0.0, 0.1, z);
            assert_eq!(
                after.lines().next().unwrap(),
                format!(
                    "; Calib_Retraction_tower: Z_HEIGHT: {}, length:{}",
                    edge_gcode::calib::general(z, 6),
                    edge_gcode::calib::general(length, 6)
                ),
                "{machine} at {z}"
            );
        }
        // Every retraction on a layer is that layer's length, but for the
        // one at the layer change, made before the new length is set.
        for (index, (z, pulls)) in by_layer.iter().enumerate().skip(1) {
            let length = edge_gcode::calib::tower_retraction(0.0, 0.1, tops[index]);
            let previous = edge_gcode::calib::tower_retraction(0.0, 0.1, tops[index - 1]);
            for pull in pulls {
                assert!(
                    (pull - length).abs() < 1e-5 || (pull - previous).abs() < 1e-5,
                    "{machine} at {z}: {pull} for {length}"
                );
            }
        }
        // The top: at 11.4, 11 steps of 0.1 (1.1 mm).
        let (z, pulls) = by_layer.last().unwrap();
        assert!((z - 11.4).abs() < 1e-6);
        assert!(
            pulls.iter().any(|pull| (pull - 1.1).abs() < 1e-5),
            "{pulls:?}"
        );
    }
}
