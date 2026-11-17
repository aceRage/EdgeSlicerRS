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
    // A printer with no usual filament: the first made for it.
    let filament = filament
        .or_else(|| {
            shelf
                .filaments_for(machine)
                .first()
                .map(|name| (*name).to_owned())
        })
        .unwrap();
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
        // A list as the pages hold it, its values by commas.
        let value = if value.contains(',') {
            Value::Many(value.split(',').map(str::to_owned).collect())
        } else {
            Value::One(value.clone())
        };
        settings.insert(key.clone(), value);
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
    // A setting of the printer's own profile that nothing reads (the
    // Ender-3 V3 SE's file_start_gcode, disable_m73) is said all the
    // same, and is no concern of the calibration's unless it sets it.
    let set_here = |note: &String| {
        !note.contains(": not followed; the print is made as if it were")
            || (made.process.iter())
                .chain(&made.filament)
                .chain(&made.printer)
                .any(|(key, _)| {
                    note.strip_prefix(key.as_str())
                        .is_some_and(|rest| rest.starts_with([' ', ':']))
                })
    };
    let notes = sliced.gaps().into_iter().filter(set_here).collect();
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

/// The first F word of a layer's extruding moves (an unretraction is
/// not one) after the calibration's place (the fan marker), in mm/s, and
/// the layer's top.
fn wall_speeds(gcode: &str) -> Vec<(f64, Option<f64>)> {
    layers(gcode)
        .iter()
        .map(|text| {
            let z: f64 = text
                .lines()
                .find_map(|line| line.strip_prefix(";Z:"))
                .unwrap()
                .parse()
                .unwrap();
            let after = text
                .split_once(";_SET_FAN_SPEED_CHANGING_LAYER\n")
                .map_or(*text, |(_, rest)| rest);
            let feed = |line: &str| {
                line.split_whitespace()
                    .find_map(|word| word.strip_prefix('F'))
                    .map(|f| f.parse::<f64>().unwrap() / 60.0)
            };
            // A feedrate set on a line of its own before the move counts
            // (a vase's first move too short to keep leaves it so).
            let mut set = None;
            let speed = after
                .lines()
                .filter(|line| line.starts_with("G1 "))
                .find_map(|line| {
                    if line.contains(" X") && line.contains(" E") && !line.contains(" E-") {
                        Some(feed(line).or(set))
                    } else {
                        if !line.contains(" X") && !line.contains(" E") && !line.contains(" Z") {
                            set = feed(line).or(set);
                        }
                        None
                    }
                });
            let speed = speed.flatten();
            (z, speed)
        })
        .collect()
}

#[test]
fn the_max_flowrate_tower_speeds_its_wall_up_with_the_height() {
    for (vendor, machine) in [
        ("Snapmaker", "Snapmaker U1 (0.4 nozzle)"),
        ("Creality", "Creality Ender-3 V3 SE 0.4 nozzle"),
    ] {
        let chosen = presets(vendor, machine);
        let settings = edge_engine::settled(&chosen, &Overrides::default(), None).unwrap();
        let bed = bed(&settings);
        // The dialog's defaults: 5 to 20 mm³/s by 0.5.
        let params = CalibParams::new(CalibMode::VolumetricSpeedTower, 5.0, 20.0, 0.5);
        let made = edge_engine::calib::max_flowrate(&params, &settings, Some(&bed)).unwrap();
        assert_eq!(made.name, "Max volumetric speed test");
        // Plater.cpp: a 0.7 mm line 0.32 mm high (Flow.cpp `mm3_per_mm`,
        // in single precision: 0.32 × (0.7 - 0.32 × (1 - π/4)) =
        // 0.20203 mm³/mm), times the filament's flow ratio.
        let flow_ratio = settings.floats("filament_flow_ratio").unwrap().unwrap()[0].unwrap();
        let mm3 = edge_engine::calib::line_mm3_per_mm(0.7, 0.32) * flow_ratio;
        let speeds = made.params.clone().unwrap();
        assert!((speeds.start - 5.0 / mm3).abs() < 1e-9);
        assert!((speeds.step - 0.5 / mm3).abs() < 1e-9);
        let process: std::collections::BTreeMap<&str, &str> = made
            .process
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(process["spiral_mode"], "1");
        assert_eq!(process["timelapse_type"], "0");
        assert!(
            made.filament
                .contains(&("slow_down_layer_time".into(), "0".into()))
        );
        let (gcode, notes) = slice_made(&chosen, &made);
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("max_flowrate_{vendor}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        // Nothing it sets goes unfollowed.
        assert!(notes.is_empty(), "{machine}: {notes:?}");
        // Cut at (20 - 5 + 1) / 0.5 = 32 mm: 100 layers of 0.32.
        let by_layer = wall_speeds(&gcode);
        let top = by_layer.last().unwrap().0;
        assert!((top - 32.0).abs() < 0.33, "{machine}: {top}");
        // From the second layer on, the wall at `round(start + z × step)`.
        for (z, speed) in by_layer.iter().skip(1) {
            let expected = edge_gcode::calib::volumetric_tower_speed(speeds.start, speeds.step, *z);
            assert_eq!(*speed, Some(expected), "{machine} at {z}");
        }
    }
}

#[test]
fn the_vfa_tower_steps_its_wall_speed_block_by_block() {
    for (vendor, machine) in [
        ("Snapmaker", "Snapmaker U1 (0.4 nozzle)"),
        ("Creality", "Creality Ender-3 V3 SE 0.4 nozzle"),
    ] {
        let chosen = presets(vendor, machine);
        let settings = edge_engine::settled(&chosen, &Overrides::default(), None).unwrap();
        let bed = bed(&settings);
        let calib = reference("resources/calib");
        // The dialog's defaults: 40 to 200 mm/s by 10, sized to the nozzle.
        let params = CalibParams::new(CalibMode::VfaTower, 40.0, 200.0, 10.0);
        let made = edge_engine::calib::vfa(&params, &settings, &calib, Some(&bed)).unwrap();
        assert_eq!(made.name, "VFA test");
        let process: std::collections::BTreeMap<&str, &str> = made
            .process
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(process["spiral_mode"], "1");
        assert_eq!(process["bottom_shell_layers"], "1");
        assert_eq!(process["bottom_shell_thickness"], "0");
        assert_eq!(process["initial_layer_print_height"], "0.2");
        let (gcode, notes) = slice_made(&chosen, &made);
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("vfa_{vendor}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert!(notes.is_empty(), "{machine}: {notes:?}");
        // 17 blocks of 5 mm (EPSILON under 85): 425 layers of 0.2, the
        // last laid again after itself, flat, its layer change and all
        // (SpiralVase.cpp: `transition_gcode` takes every line of it).
        assert!(gcode.contains("; total layer number: 425"), "{machine}");
        let mut by_layer = wall_speeds(&gcode);
        assert_eq!(by_layer.len(), 426, "{machine}");
        assert_eq!(by_layer[424].0, by_layer[425].0);
        by_layer.truncate(425);
        // What the filament can melt caps the wall (GCode.cpp `_extrude`).
        let limit = |speed: f64| speed;
        let count = by_layer.len();
        let mut seen = std::collections::BTreeSet::new();
        for (index, (z, speed)) in by_layer.iter().enumerate().skip(1) {
            let expected = edge_gcode::calib::vfa_tower_speed(index, count, 40.0, 200.0, 10.0);
            let speed = speed.unwrap();
            assert!(
                speed <= limit(expected) + 1e-9,
                "{machine} at {z}: {speed} for {expected}"
            );
            seen.insert(expected as i64);
            if expected <= 100.0 {
                assert_eq!(speed, expected, "{machine} at {z}");
            }
        }
        // Every band, 25 layers each: 40, 50, … 200.
        assert_eq!(seen.len(), 17, "{seen:?}");
        // Band 1 starts at layer floor(424 / 17) + 1 = 25: 50 mm/s.
        assert_eq!(
            edge_gcode::calib::vfa_tower_speed(25, count, 40.0, 200.0, 10.0),
            50.0
        );
        assert_eq!(
            edge_gcode::calib::vfa_tower_speed(24, count, 40.0, 200.0, 10.0),
            40.0
        );
    }
}

/// Makes a calibration for the printer, with `base` set over its presets
/// first (as the shown project's own changes would be), and slices it.
fn made_and_sliced(
    chosen: &Chosen,
    base: &Settings,
    make: impl Fn(&Settings, &Bed) -> Result<CalibProject, edge_engine::Failure>,
) -> Result<(CalibProject, String), edge_engine::Failure> {
    let over = Overrides {
        settings: base.clone(),
        ..Overrides::default()
    };
    let settings = edge_engine::settled(chosen, &over, None)?;
    let bed = bed(&settings);
    let made = make(&settings, &bed)?;
    let loaded = made
        .model
        .place(made.model.plates().first().map(|plate| plate.id()))?;
    let mut set = overrides(&made);
    for (key, value) in base.iter() {
        if set.settings.get(key).is_none() {
            set.settings.insert(key.to_owned(), value.clone());
        }
    }
    let job = edge_engine::prepare(loaded, chosen, &set)?;
    let sliced = edge_engine::slice(job, &Unwatched)?;
    let text = sliced.gcode("1970-01-01 at 00:00:01 UTC")?.text;
    Ok((made, String::from_utf8(text).unwrap()))
}

/// The lines after each layer's fan marker that a calibration wrote,
/// those starting with one of `starts`; and whether each came before the
/// layer's first extruding move.
fn per_layer_lines<'a>(gcode: &'a str, starts: &[&str]) -> Vec<Vec<&'a str>> {
    layers(gcode)
        .iter()
        .map(|text| {
            let after = text
                .split_once(";_SET_FAN_SPEED_CHANGING_LAYER\n")
                .map_or(*text, |(_, rest)| rest);
            let mut found = Vec::new();
            for line in after.lines() {
                if line.starts_with("G1 ") && line.contains(" X") && line.contains(" E") {
                    break;
                }
                if starts.iter().any(|start| line.starts_with(start)) {
                    found.push(line);
                }
            }
            found
        })
        .collect()
}

#[test]
fn the_input_shaping_frequency_test_spreads_the_frequency_over_the_layers() {
    for (vendor, machine, shaper, first_line) in [
        (
            "Snapmaker",
            "Snapmaker U1 (0.4 nozzle)",
            "MZV",
            "SET_INPUT_SHAPER SHAPER_TYPE=MZV DAMPING_RATIO_X=0.150 DAMPING_RATIO_Y=0.150",
        ),
        (
            "Creality",
            "Creality Ender-3 V3 SE 0.4 nozzle",
            "ZV",
            "M593 D0.150",
        ),
        // RepRapFirmware: no shipped printer of it has filaments of its
        // own vendor's here, so the U1 told it is one.
        (
            "RepRapFirmware",
            "Snapmaker U1 (0.4 nozzle)",
            "MZV",
            "M593 P\"MZV\" S0.150",
        ),
    ] {
        let rrf = vendor == "RepRapFirmware";
        let chosen = presets(if rrf { "Snapmaker" } else { vendor }, machine);
        let mut base = Settings::default();
        if rrf {
            base.insert(
                "gcode_flavor".to_owned(),
                Value::One("reprapfirmware".to_owned()),
            );
        }
        // The dialog's defaults: 15 to 110 Hz on both axes, damping 0.15.
        let mut params = CalibParams::new(CalibMode::InputShapingFrequency, 0.15, 1.0, 0.1);
        params.freq_x = (15.0, 110.0);
        params.freq_y = (15.0, 110.0);
        params.shaper_type = shaper.to_owned();
        let calib = reference("resources/calib");
        let (made, gcode) = made_and_sliced(&chosen, &base, |settings, bed| {
            edge_engine::calib::input_shaping(&params, settings, &calib, Some(bed))
        })
        .unwrap();
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("shaping_freq_{vendor}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(made.name, "Input shaping Frequency test");
        let process: std::collections::BTreeMap<&str, &str> = made
            .process
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(process["spiral_mode"], "1");
        assert_eq!(process["default_jerk"], "0");
        let count: usize = gcode
            .lines()
            .find_map(|line| line.strip_prefix("; total layer number: "))
            .unwrap()
            .parse()
            .unwrap();
        let lines = per_layer_lines(&gcode, &["SET_INPUT_SHAPER", "M593"]);
        assert!(lines.len() >= count, "{machine}");
        let marlin = vendor == "Creality";
        for (index, found) in lines.iter().enumerate().take(count) {
            if index == 1 {
                assert_eq!(found, &[first_line], "{machine}");
                continue;
            }
            // From 15 to 110 over the layers from the third on, in single
            // precision (GCode.cpp `interpolate_value_across_layers`).
            let freq = edge_gcode::calib::spread_layer_value(index, count, 15.0, 110.0);
            let expected = if rrf || marlin {
                format!("M593 F{freq:.2}")
            } else {
                format!("SET_INPUT_SHAPER SHAPER_FREQ_X={freq:.2} SHAPER_FREQ_Y={freq:.2}")
            };
            assert_eq!(found, &[expected.as_str()], "{machine} layer {index}");
        }
        // Worked by hand: the first layers at 15 Hz, the last at 110; the
        // middle of n layers, index 2 + (n - 3) / 2, at 62.5.
        assert_eq!(
            edge_gcode::calib::spread_layer_value(0, 103, 15.0, 110.0),
            15.0
        );
        assert_eq!(
            edge_gcode::calib::spread_layer_value(2, 103, 15.0, 110.0),
            15.0
        );
        assert_eq!(
            edge_gcode::calib::spread_layer_value(52, 103, 15.0, 110.0),
            62.5
        );
        assert_eq!(
            edge_gcode::calib::spread_layer_value(102, 103, 15.0, 110.0),
            110.0
        );
    }
    // A Bambu Lab printer (Marlin before 2 to the reference's writer)
    // takes no input shaping: refused, as the reference's writer refuses.
    let chosen = presets("BBL", "Bambu Lab X1 Carbon 0.4 nozzle");
    let params = CalibParams::new(CalibMode::InputShapingFrequency, 0.15, 1.0, 0.1);
    let calib = reference("resources/calib");
    let refused = made_and_sliced(&chosen, &Settings::default(), |settings, bed| {
        edge_engine::calib::input_shaping(&params, settings, &calib, Some(bed))
    })
    .unwrap_err()
    .to_string();
    assert!(
        refused.starts_with("Input shaping is not supported by Marlin < 2.1.2."),
        "{refused}"
    );
}

#[test]
fn the_input_shaping_damping_test_spreads_the_damping_over_the_layers() {
    for (vendor, machine, shaper, first_lines) in [
        (
            "Snapmaker",
            "Snapmaker U1 (0.4 nozzle)",
            "ZV",
            vec![
                "SET_INPUT_SHAPER SHAPER_TYPE=ZV SHAPER_FREQ_X=30.00",
                "SET_INPUT_SHAPER SHAPER_TYPE=ZV SHAPER_FREQ_Y=42.50",
            ],
        ),
        (
            "Creality",
            "Creality Ender-3 V3 SE 0.4 nozzle",
            "ZV",
            vec!["M593 X F30.00", "M593 Y F42.50"],
        ),
    ] {
        let chosen = presets(vendor, machine);
        // Frequencies 30 and 42.5 Hz, damping 0 to 0.4 (the dialog's).
        let mut params = CalibParams::new(CalibMode::InputShapingDamping, 0.0, 0.4, 0.1);
        params.freq_x = (30.0, 1.0);
        params.freq_y = (42.5, 1.0);
        params.shaper_type = shaper.to_owned();
        params.test_model = 1;
        let calib = reference("resources/calib");
        let (made, gcode) = made_and_sliced(&chosen, &Settings::default(), |settings, bed| {
            edge_engine::calib::input_shaping(&params, settings, &calib, Some(bed))
        })
        .unwrap();
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("shaping_damp_{vendor}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(made.name, "Input shaping Damping test");
        let count: usize = gcode
            .lines()
            .find_map(|line| line.strip_prefix("; total layer number: "))
            .unwrap()
            .parse()
            .unwrap();
        let lines = per_layer_lines(&gcode, &["SET_INPUT_SHAPER", "M593"]);
        for (index, found) in lines.iter().enumerate().take(count) {
            if index == 1 {
                assert_eq!(found, &first_lines, "{machine}");
                continue;
            }
            let damp = edge_gcode::calib::spread_layer_value(index, count, 0.0, 0.4);
            // A damping of 0 is not written (`damp > 0`): nothing at all.
            let expected: Vec<String> = if damp > 0.0 {
                vec![if vendor == "Creality" {
                    format!("M593 D{damp:.3}")
                } else {
                    format!("SET_INPUT_SHAPER DAMPING_RATIO_X={damp:.3} DAMPING_RATIO_Y={damp:.3}")
                }]
            } else {
                Vec::new()
            };
            assert_eq!(found, &expected, "{machine} layer {index}");
        }
        // The top layer at 0.4.
        assert_eq!(
            edge_gcode::calib::spread_layer_value(count - 1, count, 0.0, 0.4),
            0.4
        );
    }
}

#[test]
fn the_cornering_test_spreads_the_corner_speed_over_the_layers() {
    let jd = {
        let mut base = Settings::default();
        base.insert(
            "machine_max_junction_deviation".to_owned(),
            Value::Many(vec!["0.08".to_owned(), "0".to_owned()]),
        );
        base
    };
    for (label, vendor, machine, base, start, end) in [
        (
            "klipper",
            "Snapmaker",
            "Snapmaker U1 (0.4 nozzle)",
            Settings::default(),
            1.0,
            15.0,
        ),
        (
            "marlin",
            "Creality",
            "Creality Ender-3 V3 SE 0.4 nozzle",
            Settings::default(),
            1.0,
            15.0,
        ),
        (
            "junction",
            "Creality",
            "Creality Ender-3 V3 SE 0.4 nozzle",
            jd,
            0.0,
            0.25,
        ),
        (
            "bambu",
            "BBL",
            "Bambu Lab X1 Carbon 0.4 nozzle",
            Settings::default(),
            1.0,
            15.0,
        ),
    ] {
        let chosen = presets(vendor, machine);
        let mut params = CalibParams::new(CalibMode::Cornering, start, end, 0.1);
        params.test_model = 2;
        let calib = reference("resources/calib");
        let (made, gcode) = made_and_sliced(&chosen, &base, |settings, bed| {
            edge_engine::calib::cornering(&params, settings, &calib, Some(bed))
        })
        .unwrap();
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("cornering_{label}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(made.name, "Cornering test");
        let printer: std::collections::BTreeMap<&str, &str> = made
            .printer
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if label == "junction" {
            assert_eq!(printer["machine_max_junction_deviation"], "0.25");
        } else {
            assert_eq!(printer["machine_max_jerk_x"], "15");
        }
        // Input shaping turned off for the test but on Klipper.
        assert_eq!(
            printer.get("input_shaping_type").copied(),
            (label != "klipper").then_some("Disable")
        );
        if label == "marlin" || label == "junction" {
            // Told before the print (GCode.cpp `print_machine_envelope`).
            assert!(gcode.contains("\nM593 F0.00 D0.000\n"), "{label}");
        }
        let count: usize = gcode
            .lines()
            .find_map(|line| line.strip_prefix("; total layer number: "))
            .unwrap()
            .parse()
            .unwrap();
        let lines = per_layer_lines(&gcode, &["SET_VELOCITY_LIMIT SQUARE", "M205"]);
        let mut last = 0.0_f64;
        for (index, found) in lines.iter().enumerate().take(count) {
            let value = f64::from(edge_gcode::calib::spread_layer_value(
                index,
                count,
                start as f32,
                end as f32,
            ));
            let shown = edge_gcode::calib::general(value, 6);
            let expected: Vec<String> = match label {
                // Every layer, to three places, held to the machine's.
                "junction" if value > 0.0 => vec![format!("M205 J{value:.3}")],
                "junction" => Vec::new(),
                // Only when it changes (`m_last_jerk`).
                _ if (value - last).abs() < 1e-4 => Vec::new(),
                "klipper" => vec![format!("SET_VELOCITY_LIMIT SQUARE_CORNER_VELOCITY={shown}")],
                "marlin" => vec![format!("M205 X{shown} Y{shown}")],
                _ => vec![format!("M205 X{shown} Y{shown} Z3 E2.5")],
            };
            if label != "junction" && (value - last).abs() >= 1e-4 {
                last = value;
            }
            assert_eq!(found, &expected, "{label} layer {index}");
        }
    }
    // Worked by hand: 1 to 15 over 100 layers, the third at 1 and each
    // after 14 / 97 more; the last at 15.
    assert_eq!(
        edge_gcode::calib::spread_layer_value(3, 100, 1.0, 15.0),
        1.0 + 14.0 / 97.0
    );
    assert_eq!(
        edge_gcode::calib::spread_layer_value(99, 100, 1.0, 15.0),
        15.0
    );
    let mut last = 0.0;
    assert_eq!(
        edge_gcode::calib::jerk_line(
            12.6,
            &mut last,
            edge_gcode::JerkForm::Marlin,
            [12.0, 12.0],
            None,
            false
        ),
        "M205 X12 Y12\n"
    );
    assert_eq!(
        edge_gcode::calib::junction_deviation_line(0.3, 0.25, true),
        "M205 J0.250 ; Junction Deviation\n"
    );
}

/// The stringing grid's per-copy and per-layer lines, read back.
fn grid_params(labels: bool) -> CalibParams {
    let mut params = CalibParams::new(CalibMode::StringingGrid, 230.0, 205.0, 5.0);
    params.grid.print_labels = labels;
    params.grid.fillet_radius = edge_engine::calib::stringing::max_fillet_radius();
    params
}

#[test]
fn the_stringing_grid_sets_its_bands_and_its_columns_retractions() {
    use edge_engine::calib::stringing;
    for (label, vendor, machine, labels) in [
        ("klipper", "Snapmaker", "Snapmaker U1 (0.4 nozzle)", false),
        (
            "marlin",
            "Creality",
            "Creality Ender-3 V3 SE 0.4 nozzle",
            true,
        ),
    ] {
        let chosen = presets(vendor, machine);
        let params = grid_params(labels);
        let (made, gcode) = made_and_sliced(&chosen, &Settings::default(), |settings, bed| {
            stringing::stringing_grid(&params, settings, Some(bed))
        })
        .unwrap();
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("stringing_{label}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(made.name, "Stringing temp × retraction");
        // Eight columns of 0.2 to 1.6 mm, and the label tower.
        let names: Vec<&str> = made.model.objects().iter().map(|o| o.name()).collect();
        let mut expected: Vec<String> = (0..8).map(stringing::column_name).collect();
        if labels {
            expected.push("SG_label".to_owned());
        }
        assert_eq!(names, expected, "{label}");
        // Six bands of 8 mm on a 1 mm base: 49 mm, 245 layers of 0.2.
        let layer_texts = layers(&gcode);
        assert_eq!(layer_texts.len(), 245, "{label}");
        let grid = stringing::for_writer(
            &params,
            &edge_engine::settled(&chosen, &Overrides::default(), None).unwrap(),
        );
        let mut last_band = None;
        for (index, text) in layer_texts.iter().enumerate() {
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
            let bands: Vec<&str> = after
                .lines()
                .filter(|line| line.starts_with("; Calib_TempRetract: band="))
                .collect();
            if index == 0 {
                assert!(bands.is_empty());
            } else {
                let band = grid.band(z - 0.1);
                if last_band != Some(band) {
                    let temperature = grid.temperature(band).round();
                    assert_eq!(
                        bands,
                        vec![format!(
                            "; Calib_TempRetract: band={band} temp={temperature}"
                        )],
                        "{label} at {z}"
                    );
                    let next = after
                        .split_once(bands[0])
                        .unwrap()
                        .1
                        .lines()
                        .nth(1)
                        .unwrap();
                    // One filament: no tool named (GCodeWriter.cpp
                    // `set_temperature`, `multiple_extruders`).
                    assert_eq!(
                        next,
                        format!(
                            "M109 S{temperature} ; set nozzle temperature and wait for it to be reached"
                        ),
                        "{label}"
                    );
                    last_band = Some(band);
                } else {
                    assert!(bands.is_empty(), "{label} at {z}");
                }
            }
            // Each column's retraction from its start: every pull while
            // it prints is its length (the travel to it keeps the one
            // before, which is longer only on the layer's first column).
            let mut length = None;
            let mut columns = 0;
            for line in text.lines() {
                // The grid's own retraction is put back after its last
                // column; what comes after (the end G-code) is not a
                // column's. The label is printed first, as the reference
                // orders the copies, so the last column ends the layer.
                if line.starts_with("; Calib_TempRetract: restore") {
                    length = None;
                    continue;
                }
                if let Some(rest) = line.strip_prefix("; Calib_TempRetract: col=") {
                    let (column, value) = rest.split_once(" retract=").unwrap();
                    if column == "none" {
                        length = None;
                        continue;
                    }
                    let column: usize = column.parse().unwrap();
                    let value: f64 = value.parse().unwrap();
                    assert!((value - (0.2 + 0.2 * column as f64)).abs() < 1e-9);
                    length = Some(value);
                    columns += 1;
                } else if let (Some(length), Some(rest)) = (length, line.strip_prefix("G1 E-")) {
                    let pulled: f64 = rest.split_whitespace().next().unwrap().parse().unwrap();
                    assert!(
                        pulled <= length + 1e-5,
                        "{label} at {z}: {pulled} for {length}"
                    );
                }
            }
            assert_eq!(columns, 8, "{label} at {z}");
        }
        // The second layer's temperature is the grid's, not the filament's.
        let second = layer_texts[1];
        assert_eq!(second.matches("M104 ").count(), 0, "{label}");
        assert!(
            gcode.contains("; Calib_TempRetract: restore retract="),
            "{label}"
        );
    }
}

#[test]
fn the_stringing_grid_dialog_defaults_and_checks_are_the_references() {
    use edge_engine::calib::stringing;
    // PLA at 220 within 190-240: 225, 215, 230, 210, ... from the middle out.
    let (params, needs) = stringing::default_params(220.0, 190.0, 240.0, 5.0, 0.0);
    assert!(!needs);
    assert_eq!((params.start, params.end), (240.0, 195.0));
    assert_eq!(stringing::band_count(&params), 10);
    // A range of one band asks for one.
    let (params, needs) = stringing::default_params(220.0, 220.0, 220.0, 5.0, 0.0);
    assert!(needs);
    assert_eq!((params.start, params.end), (220.0, 220.0));
    // Nonsense: 230 to 205.
    let (params, needs) = stringing::default_params(0.0, 190.0, 240.0, 5.0, 0.0);
    assert!(!needs);
    assert_eq!((params.start, params.end), (230.0, 205.0));
    let params = grid_params(false);
    assert_eq!(stringing::params_valid(&params, 270.0), Ok(()));
    // 270 mm: (270 - 30 - 35) / 12 = 17 columns; a 100 mm bed fits 2.
    assert_eq!(stringing::max_columns(270.0), 17);
    let error = stringing::params_valid(&params, 100.0).unwrap_err();
    assert_eq!(
        error.message,
        "Grid does not fit the bed (max %1% columns at 12 mm pitch)"
    );
    assert_eq!(error.value.as_deref(), Some("2"));
    let mut wide = grid_params(false);
    wide.grid.fillet_radius = 4.0;
    let error = stringing::params_valid(&wide, 270.0).unwrap_err();
    assert_eq!(error.value.as_deref(), Some("3"));
    let mut odd = grid_params(false);
    odd.end = 207.0;
    assert_eq!(
        stringing::params_valid(&odd, 270.0).unwrap_err().message,
        "The temperature range must be a multiple of the temperature step"
    );
    assert_eq!(stringing::retract_label(0.2), "0.2");
    assert_eq!(stringing::retract_label(10.0), "10");
    assert_eq!(edge_gcode::calib::stringing_column("SG_col_07"), Some(7));
    assert_eq!(edge_gcode::calib::stringing_column("SG_col_3abc"), None);
    assert_eq!(edge_gcode::calib::stringing_column("SG_label"), None);
    assert_eq!(
        edge_gcode::calib::temperature_command(215, true, None, "reprapfirmware"),
        "G10 S215 ; set nozzle temperature\nM116 ; wait for temperature to be reached\n"
    );
    assert_eq!(
        edge_gcode::calib::temperature_command(215, true, None, "sailfish"),
        ""
    );
}

#[test]
fn the_ironing_flow_dialog_checks_and_labels_are_the_references() {
    use edge_engine::calib::ironing_flow::{check, label, mark_step};
    // Marks at 1, 2, 5, 10 or 20 %, at least 10 mm apart.
    assert_eq!(mark_step(0.0, 70.0, 140.0), 5.0);
    assert_eq!(mark_step(0.0, 10.0, 140.0), 1.0);
    assert_eq!(mark_step(0.0, 100.0, 50.0), 20.0);
    assert_eq!(mark_step(0.0, 100.0, 40.0), 0.0);
    assert_eq!(label(35.0), "35%");
    assert_eq!(label(12.5), "12.5%");
    assert_eq!(label(12.345), "12.35%");
    assert_eq!(label(0.0), "0%");
    let bed = [0.0, 0.0, 270.0, 270.0];
    assert_eq!(check(0.0, 70.0, 140.0, 0.4, bed), Ok(()));
    for (start, end, length) in [
        (-1.0, 70.0, 140.0),
        (30.0, 30.0, 140.0),
        (0.0, 101.0, 140.0),
        (0.0, 70.0, 9.0),
    ] {
        assert!(
            check(start, end, length, 0.4, bed)
                .unwrap_err()
                .message
                .starts_with("Please input valid values")
        );
    }
    let error = check(0.0, 100.0, 40.0, 0.4, bed).unwrap_err();
    assert_eq!(error.values, vec![50]);
    let error = check(0.0, 70.0, 300.0, 0.4, bed).unwrap_err();
    assert_eq!(error.values, vec![300, 310, 250]);
    assert!(
        check(0.0, 70.0, 140.0, 0.4, [0.0, 0.0, 270.0, 60.0])
            .unwrap_err()
            .message
            .contains("in Y")
    );
    assert_eq!(
        edge_engine::calib::ironing_flow::filament_name("Snapmaker PLA @U1"),
        "Snapmaker PLA"
    );
}

/// The ironing moves of the last layer: each extruding move's start, end
/// and filament fed.
fn ironing_moves(gcode: &str) -> Vec<([f64; 2], [f64; 2], f64)> {
    let last = layers(gcode).last().copied().unwrap();
    let mut at = [0.0, 0.0];
    // Where the nozzle is at the layer's start: the last move before it.
    let pieces: Vec<&str> = gcode.split(";LAYER_CHANGE\n").collect();
    for line in pieces[pieces.len() - 2].lines() {
        for word in line.split_whitespace() {
            if let Some(x) = word.strip_prefix('X') {
                at[0] = x.parse().unwrap();
            } else if let Some(y) = word.strip_prefix('Y') {
                at[1] = y.parse().unwrap();
            }
        }
    }
    let mut ironing = false;
    let mut moves = Vec::new();
    for line in last.lines() {
        if let Some(kind) = line.strip_prefix(";TYPE:") {
            ironing = kind == "Ironing";
            continue;
        }
        if !(line.starts_with("G1 ") || line.starts_with("G0 ")) {
            continue;
        }
        let mut to = at;
        let mut fed = 0.0;
        for word in line.split(';').next().unwrap().split_whitespace() {
            if let Some(x) = word.strip_prefix('X') {
                to[0] = x.parse().unwrap();
            } else if let Some(y) = word.strip_prefix('Y') {
                to[1] = y.parse().unwrap();
            } else if let Some(e) = word.strip_prefix('E') {
                fed = e.parse().unwrap();
            }
        }
        if ironing && to != at {
            moves.push((at, to, fed));
        }
        at = to;
    }
    moves
}

#[test]
fn the_ironing_flow_test_ramps_its_ironing_along_both_bands() {
    use edge_engine::calib::ironing_flow;
    let fonts = reference("resources/fonts");
    for (label, vendor, machine) in [
        ("klipper", "Snapmaker", "Snapmaker U1 (0.4 nozzle)"),
        ("marlin", "Creality", "Creality Ender-3 V3 SE 0.4 nozzle"),
    ] {
        let chosen = presets(vendor, machine);
        let params = CalibParams::new(CalibMode::IroningFlow, 0.0, 70.0, 140.0);
        let (mut made, _) = made_and_sliced(&chosen, &Settings::default(), |settings, bed| {
            ironing_flow::ironing_flow(&params, settings, &fonts, Some(bed))
        })
        .unwrap();
        assert_eq!(made.name, "Ironing flow test");
        let object = &made.model.objects()[0];
        assert_eq!(object.name(), "Ironing flow test");
        let over: Vec<(&str, &str)> = made
            .process
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            over,
            vec![
                ("ironing_type", "topmost"),
                ("ironing_pattern", "rectilinear"),
                ("ironing_angle_fixed", "1"),
                ("ironing_angle", "90")
            ]
        );
        // Named at the slice after the filament, the test stays where it was.
        let settings = edge_engine::settled(&chosen, &overrides(&made), None).unwrap();
        let bed = bed(&settings);
        let copy = made.model.objects()[0].copies()[0].id();
        let before = made.model.in_scene(Some(&bed), copy).unwrap().1.unwrap();
        let names = vec![chosen.filaments[0].0.clone()];
        let changed = ironing_flow::name_for_slice(
            &mut made.model,
            Some(&bed),
            made.params.as_ref().unwrap(),
            &settings,
            &names,
            &fonts,
        )
        .unwrap();
        assert!(changed, "{label}");
        let again = ironing_flow::name_for_slice(
            &mut made.model,
            Some(&bed),
            made.params.as_ref().unwrap(),
            &settings,
            &names,
            &fonts,
        )
        .unwrap();
        assert!(!again);
        let [low, high] = made.model.in_scene(Some(&bed), copy).unwrap().1.unwrap();
        for axis in 0..3 {
            assert!(
                (low[axis] - before[0][axis]).abs() < 1e-4
                    && (high[axis] - before[1][axis]).abs() < 1e-4,
                "{label} {low:?} {before:?}"
            );
        }
        // 140 mm and 5 mm at each end; ten layers of 0.12 over the first.
        let first: f64 = settings
            .float("initial_layer_print_height")
            .unwrap()
            .unwrap();
        assert!((high[0] - low[0] - 150.0).abs() < 1e-3, "{label}");
        assert!(
            (high[2] - low[2] - (first + 9.0 * 0.12)).abs() < 1e-4,
            "{label}"
        );
        let (gcode, notes) = slice_made(&chosen, &made);
        assert!(notes.is_empty(), "{label}: {notes:?}");
        if let Ok(dir) = std::env::var("EDGE_CALIB_DUMP") {
            std::fs::write(
                std::path::Path::new(&dir).join(format!("ironing_flow_{label}.gcode")),
                &gcode,
            )
            .unwrap();
        }
        assert_eq!(layers(&gcode).len(), 10, "{label}");
        // Every long ironing line feeds the ramp's flow at its X: the
        // percent of a line 0.12 high spread over the spacing, from 0 % at
        // 5 mm in from the left to 70 % at 5 mm in from the right on the
        // front band, the other way on the band behind.
        let spacing = settings.float("ironing_spacing").unwrap().unwrap();
        let diameter = settings.floats("filament_diameter").unwrap().unwrap()[0].unwrap();
        let ratio = settings.floats("filament_flow_ratio").unwrap().unwrap()[0].unwrap()
            * settings.float("print_flow_ratio").unwrap().unwrap_or(1.0);
        let per_percent =
            0.12 * 0.01 * spacing * ratio / (std::f64::consts::PI * diameter * diameter / 4.0);
        let (x0, x1) = (low[0] + 5.0, high[0] - 5.0);
        let moves = ironing_moves(&gcode);
        let (mut front, mut rear) = (0, 0);
        let mut least = f64::MAX;
        let mut most: f64 = 0.0;
        for (a, b, fed) in &moves {
            let length = (b[0] - a[0]).hypot(b[1] - a[1]);
            if length < 5.0 || (b[0] - a[0]).abs() > 1e-3 {
                continue;
            }
            let t = ((a[0] - x0) / (x1 - x0)).clamp(0.0, 1.0);
            let rear_band = a[1].max(b[1]) > high[1] - 15.5;
            let percent = 70.0 * if rear_band { 1.0 - t } else { t };
            if rear_band {
                rear += 1
            } else {
                front += 1
            }
            let expected = per_percent * percent * length;
            assert!(
                (fed - expected).abs() <= 1e-5 + 1e-3 * expected,
                "{label} {a:?}->{b:?}: {fed} for {expected}"
            );
            least = least.min(percent);
            most = most.max(percent);
        }
        assert!(front > 100 && rear > 100, "{label}: {front} {rear}");
        assert!(least == 0.0 && most == 70.0, "{label}");
    }
}
