use super::*;

fn settings(json: &str) -> Settings {
    Settings::from_json(json).unwrap()
}

fn texts(settings: &Settings, key: &str) -> Vec<String> {
    settings.texts(key).unwrap().to_vec()
}

/// Two U1 presets as the Snapmaker profiles have them: a value for the
/// standard nozzle and one for the high-flow nozzle.
fn pla_and_petg() -> (Settings, Settings) {
    let pla = settings(
        r#"{"filament_flow_support": ["standard", "high_flow"],
            "nozzle_temperature": ["215", "220"], "filament_flow_ratio": ["1", "0.99"],
            "filament_retraction_length": ["nil", "1"], "fan_max_speed": ["100", "90"],
            "filament_type": ["PLA"], "filament_ramming_travel_time": ["0", "7"],
            "compatible_printers": ["U1", "U1 0.6"], "filament_vendor": ["Snapmaker"],
            "filament_flow_step_size": ["2"], "compatible_printers_condition": ""}"#,
    );
    let petg = settings(
        r#"{"filament_flow_support": ["standard", "high_flow"],
            "nozzle_temperature": ["255", "260"], "filament_flow_ratio": ["0.95", "1.01"],
            "filament_retraction_length": ["0.6", "nil"], "fan_max_speed": ["40"],
            "filament_type": ["PETG-CF"], "filament_ramming_travel_time": ["3", "8"],
            "chamber_temperature": ["40"]}"#,
    );
    (pla, petg)
}

#[test]
fn each_filament_reads_its_own_presets_standard_values() {
    let (pla, petg) = pla_and_petg();
    let (flat, notes) = for_filaments(&[("PLA", &pla), ("PETG", &petg)], 2, &[]);
    assert!(notes.is_empty());
    assert_eq!(texts(&flat, "nozzle_temperature"), ["215", "255"]);
    assert_eq!(texts(&flat, "filament_flow_ratio"), ["1", "0.95"]);
    assert_eq!(texts(&flat, "filament_retraction_length"), ["nil", "0.6"]);
    assert_eq!(texts(&flat, "fan_max_speed"), ["100", "40"]);
    assert_eq!(texts(&flat, "filament_type"), ["PLA", "PETG-CF"]);
    // Not a flow-variant setting: the first entry, whatever follows it.
    assert_eq!(texts(&flat, "filament_ramming_travel_time"), ["0", "3"]);
    assert_eq!(
        texts(&flat, "filament_flow_support"),
        ["standard", "standard"]
    );
    // What only the second has: the first takes the reference's default.
    assert_eq!(texts(&flat, "chamber_temperature"), ["0", "40"]);
    // Lists naming what a preset suits stay the first's; a single value too.
    assert_eq!(texts(&flat, "compatible_printers"), ["U1", "U1 0.6"]);
    assert_eq!(flat.text("compatible_printers_condition"), Some(""));
    assert!(flat.get("filament_flow_step_size").is_none());
}

#[test]
fn a_filament_at_high_flow_reads_its_high_flow_values() {
    let (pla, petg) = pla_and_petg();
    let flows = [Flow::Standard, Flow::HighFlow];
    let (flat, notes) = for_filaments(&[("PLA", &pla), ("PETG", &petg)], 2, &flows);
    assert!(notes.is_empty());
    assert_eq!(texts(&flat, "nozzle_temperature"), ["215", "260"]);
    assert_eq!(texts(&flat, "filament_flow_ratio"), ["1", "1.01"]);
    // A nil at high flow over a value at standard reads the standard value;
    // a list with no high-flow entry reads its only one.
    assert_eq!(texts(&flat, "filament_retraction_length"), ["nil", "0.6"]);
    assert_eq!(texts(&flat, "fan_max_speed"), ["100", "40"]);
    assert_eq!(
        texts(&flat, "filament_flow_support"),
        ["standard", "high_flow"]
    );
    // At high flow the first filament's nil reads nil only where its first
    // entry is nil too: here its standard entry is nil, its high-flow 1.
    let (flat, _) = for_filaments(&[("PLA", &pla)], 1, &[Flow::HighFlow]);
    assert_eq!(texts(&flat, "filament_retraction_length"), ["1"]);
    assert_eq!(texts(&flat, "nozzle_temperature"), ["220"]);
}

#[test]
fn filaments_past_the_presets_named_take_the_last() {
    let (pla, petg) = pla_and_petg();
    let (flat, _) = for_filaments(&[("PLA", &pla), ("PETG", &petg)], 4, &[]);
    assert_eq!(
        texts(&flat, "nozzle_temperature"),
        ["215", "255", "255", "255"]
    );
    // One preset for three filaments: the same value three times, never
    // its high-flow value for the second.
    let (flat, _) = for_filaments(&[("PLA", &pla)], 3, &[]);
    assert_eq!(texts(&flat, "nozzle_temperature"), ["215", "215", "215"]);
    assert_eq!(texts(&flat, "filament_flow_ratio"), ["1", "1", "1"]);
}

#[test]
fn high_flow_for_a_preset_without_it_is_said_and_reads_standard() {
    let plain = settings(r#"{"nozzle_temperature": ["210"], "filament_type": ["PLA"]}"#);
    let (flat, notes) = for_filaments(&[("Plain PLA", &plain)], 2, &[Flow::HighFlow]);
    assert_eq!(texts(&flat, "nozzle_temperature"), ["210", "210"]);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("filament 1") && notes[0].contains("\"Plain PLA\""));
}

#[test]
fn a_bambu_preset_names_its_variants_by_extruder_variant() {
    // Bambu PLA Basic @BBL H2D: no filament_flow_support of its own.
    let bambu = settings(
        r#"{"filament_extruder_variant": ["Direct Drive Standard", "Direct Drive High Flow", "Direct Drive E3D High Flow"],
            "filament_max_volumetric_speed": ["25", "40", "25"],
            "filament_retraction_length": ["0.4", "0.4", "1"],
            "nozzle_temperature": ["220", "225", "230"]}"#,
    );
    let (flat, notes) = for_filaments(&[("Bambu", &bambu)], 2, &[Flow::Standard, Flow::HighFlow]);
    assert!(notes.is_empty());
    assert_eq!(texts(&flat, "filament_max_volumetric_speed"), ["25", "40"]);
    assert_eq!(texts(&flat, "nozzle_temperature"), ["220", "225"]);
    assert_eq!(
        texts(&flat, "filament_extruder_variant"),
        ["Direct Drive Standard", "Direct Drive Standard"]
    );
    // A TPU high-flow nozzle is a variant of its own, not high flow.
    let tpu = settings(
        r#"{"filament_extruder_variant": ["Direct Drive Standard", "Direct Drive TPU High Flow"],
            "filament_max_volumetric_speed": ["12", "20"]}"#,
    );
    let (flat, notes) = for_filaments(&[("TPU", &tpu)], 1, &[Flow::HighFlow]);
    assert_eq!(texts(&flat, "filament_max_volumetric_speed"), ["12"]);
    assert_eq!(notes.len(), 1);
}

#[test]
fn flows_are_read_by_name_and_anything_else_is_refused() {
    let list = |texts: &[&str]| {
        texts
            .iter()
            .map(|text| text.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        Flow::list(&list(&["standard", "high_flow"])).unwrap(),
        [Flow::Standard, Flow::HighFlow]
    );
    assert!(Flow::list(&list(&["High Flow"])).is_err());
}

#[test]
fn a_project_packed_per_flow_variant_is_unpacked_by_filament() {
    // As the reference's application saves two U1 filaments of two
    // variants each.
    let mut project = settings(
        r#"{"filament_flow_step_size": ["2", "1", "2"],
            "filament_flow_support": ["standard", "high_flow", "standard", "standard", "high_flow"],
            "nozzle_temperature": ["215", "220", "200", "255", "260"],
            "filament_type": ["PLA", "PLA", "PETG"]}"#,
    );
    unpack_flow_variants(
        &mut project,
        &[Flow::HighFlow, Flow::HighFlow, Flow::Standard],
    );
    assert_eq!(texts(&project, "nozzle_temperature"), ["220", "200", "255"]);
    assert_eq!(
        texts(&project, "filament_flow_support"),
        ["high_flow", "standard", "standard"]
    );
    assert_eq!(texts(&project, "filament_type"), ["PLA", "PLA", "PETG"]);
    assert!(project.get("filament_flow_step_size").is_none());
    // One entry per filament already: left exactly as it is.
    let flat = settings(
        r#"{"filament_flow_step_size": ["1", "1"], "nozzle_temperature": ["215", "255"]}"#,
    );
    let mut same = flat.clone();
    unpack_flow_variants(&mut same, &[Flow::HighFlow]);
    assert_eq!(same, flat);
}

/// Bambu's H2D as its preset has it: seven (extruder, variant) slots.
fn two_extruders_by_variant(volumes: &str) -> Settings {
    settings(&format!(
        r#"{{"single_extruder_multi_material": "0", "nozzle_diameter": ["0.4", "0.4"],
            "extruder_type": ["Direct Drive", "Direct Drive"], {volumes}
            "printer_extruder_id": ["1", "1", "1", "2", "2", "2", "2"],
            "printer_extruder_variant": ["Direct Drive Standard", "Direct Drive High Flow", "Direct Drive E3D High Flow",
                "Direct Drive Standard", "Direct Drive High Flow", "Direct Drive TPU High Flow", "Direct Drive E3D High Flow"],
            "retraction_length": ["0.8", "0.9", "1.0", "0.6", "0.7", "1.2", "1.1"],
            "z_hop": ["0.4", "0.4", "0.4", "0.2", "0.2", "0.2", "0.2"],
            "nozzle_volume": ["130", "133", "133", "145", "148", "148", "148"]}}"#
    ))
}

#[test]
fn each_extruder_reads_its_own_nozzles_slot() {
    let mut printer = two_extruders_by_variant("");
    assert!(for_extruders(&mut printer).is_empty());
    assert_eq!(texts(&printer, "retraction_length"), ["0.8", "0.6"]);
    assert_eq!(texts(&printer, "z_hop"), ["0.4", "0.2"]);
    // Not a retraction setting: left as the slots are.
    assert_eq!(texts(&printer, "nozzle_volume").len(), 7);
    // A standard nozzle on the left and a high-flow one on the right.
    let mut mixed = two_extruders_by_variant(r#""nozzle_volume_type": ["Standard", "High Flow"],"#);
    assert!(for_extruders(&mut mixed).is_empty());
    assert_eq!(texts(&mixed, "retraction_length"), ["0.8", "0.7"]);
    // A variant the left extruder has no slot for: its first, said.
    let mut tpu =
        two_extruders_by_variant(r#""nozzle_volume_type": ["TPU High Flow", "TPU High Flow"],"#);
    let notes = for_extruders(&mut tpu);
    assert_eq!(texts(&tpu, "retraction_length"), ["0.8", "1.2"]);
    assert_eq!(notes.len(), 1);
    assert!(notes[0].contains("extruder 1"));
}

#[test]
fn a_printer_with_a_value_per_extruder_is_left_as_it_is() {
    // Single nozzle (the Centauri Carbon), and four nozzles of one value
    // each (the U1).
    for json in [
        r#"{"single_extruder_multi_material": "1", "nozzle_diameter": ["0.4"], "retraction_length": ["0.8"]}"#,
        r#"{"single_extruder_multi_material": "0", "nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"],
            "retraction_length": ["0.8", "0.8", "0.7", "0.8"]}"#,
        r#"{"single_extruder_multi_material": "0", "nozzle_diameter": ["0.4", "0.4"],
            "printer_extruder_id": ["1", "2"], "printer_extruder_variant": ["Direct Drive Standard", "Direct Drive Standard"],
            "retraction_length": ["0.8", "0.7"]}"#,
    ] {
        let printer = settings(json);
        let mut same = printer.clone();
        assert!(for_extruders(&mut same).is_empty());
        assert_eq!(same, printer);
    }
}
