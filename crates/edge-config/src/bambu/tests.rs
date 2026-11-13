//! The translation key by key, against Format/BambuExport.cpp,
//! BambuKeyAliases.cpp and the reference's own cases
//! (tests/libslic3r/test_bambu_3mf_export.cpp, docs/bambu-3mf-export.md).

use super::*;

fn settings(json: &str) -> Settings {
    Settings::from_json(json).unwrap()
}

/// A one-extruder project's settings and what they convert to.
fn project(json: &str) -> (Config, Context, Report) {
    let project = settings(json);
    let mut ctx = Context::of(&project);
    let mut report = Report::default();
    let out = convert_project(&project, &mut ctx, &mut report);
    (out, ctx, report)
}

fn one<'a>(out: &'a Config, key: &str) -> &'a str {
    let value = out
        .get(key)
        .unwrap_or_else(|| panic!("no {key} in {out:?}"));
    assert!(!value.vector, "{key} is a list: {value:?}");
    &value.values[0]
}

fn many<'a>(out: &'a Config, key: &str) -> Vec<&'a str> {
    let value = out.get(key).unwrap_or_else(|| panic!("no {key}"));
    assert!(value.vector, "{key} is not a list: {value:?}");
    value.values.iter().map(String::as_str).collect()
}

#[test]
fn the_table_is_the_references() {
    assert_eq!(known::KEYS.len(), 605);
    assert_eq!(known::ENUMS.len(), 86);
    assert!(
        known::KEYS.windows(2).all(|pair| pair[0].key < pair[1].key),
        "sorted, for the search"
    );
    assert_eq!(reference_version(), "02.08.02.61");
    assert_eq!(export_version(), "02.08.00.00");
    assert_eq!(application_tag(), "BambuStudio-02.08.00.00");
    // Keys Bambu Studio declares but drops when it reads a project are not
    // listed (its own `handle_legacy`).
    assert!(find_key("only_one_wall_top").is_none());
    assert!(find_key("chamber_temperature").is_none());
    assert!(find_key("top_one_wall_type").is_some());
    // Every conversion the table names is ported.
    for (ours, _, function, _) in known::ALIASES {
        if let Some(function) = function {
            let mut value = "1".to_owned();
            let _ = to_bambu(function, &mut value);
            assert!(!ours.is_empty());
        }
    }
}

#[test]
fn renamed_keys_go_out_under_bambus_names() {
    let (out, _, report) = project(
        r##"{"infill_anchor": "400%", "chamber_temperature": ["45"], "only_one_wall_top": "1",
            "support_ironing": "1", "reduce_infill_retraction": "0", "wipe_tower_wall_type": "cone",
            "filament_colour_mode": ["1"], "filament_multi_colors": ["#FF0000|#00FF00"],
            "notes": "hello", "nozzle_diameter": ["0.4"]}"##,
    );
    assert_eq!(one(&out, "sparse_infill_anchor"), "400%");
    assert_eq!(many(&out, "chamber_temperatures"), ["45"]);
    assert_eq!(one(&out, "top_one_wall_type"), "all top");
    assert_eq!(one(&out, "enable_support_ironing"), "1");
    assert_eq!(one(&out, "reduce_infill_retraction_mode"), "Disabled");
    // Bambu has no cone tower: a plain wall.
    assert_eq!(one(&out, "prime_tower_rib_wall"), "0");
    // 0/1 the other way round, and "|" as a space.
    assert_eq!(many(&out, "filament_colour_type"), ["0"]);
    assert_eq!(many(&out, "filament_multi_colour"), ["#FF0000 #00FF00"]);
    assert_eq!(one(&out, "process_notes"), "hello");
    for ours in ["infill_anchor", "only_one_wall_top", "notes"] {
        assert!(!out.contains_key(ours), "{ours}");
        assert!(report.renamed.contains_key(ours), "{ours}");
    }
    assert_eq!(report.renamed["only_one_wall_top"], "top_one_wall_type");
}

#[test]
fn a_negative_ironing_angle_is_left_out_with_the_references_reason() {
    let (out, _, report) = project(r##"{"ironing_angle": "-1", "nozzle_diameter": ["0.4"]}"##);
    assert!(!out.contains_key("ironing_direction"));
    assert!(report.dropped.contains("ironing_angle"));
    assert!(report.notes.iter().any(|note| note.contains(
        "left out ironing_angle: negative ironing angle means \"default\" here; Bambu keeps its own default"
    )));
    let (out, _, _) = project(r##"{"ironing_angle": "30", "nozzle_diameter": ["0.4"]}"##);
    assert_eq!(one(&out, "ironing_direction"), "30");
}

#[test]
fn with_both_spellings_bambus_name_wins() {
    let (out, _, report) = project(
        r##"{"infill_anchor": "1", "sparse_infill_anchor": "2", "nozzle_diameter": ["0.4"]}"##,
    );
    assert_eq!(one(&out, "sparse_infill_anchor"), "2");
    assert!(report.dropped.contains("infill_anchor"));
}

#[test]
fn what_bambu_does_not_know_is_left_out_and_counted() {
    assert!(find_key("make_overhang_printable").is_none());
    let (out, _, report) = project(
        r##"{"make_overhang_printable": "1", "no_such_setting": "x", "wall_loops": "3",
            "nozzle_diameter": ["0.4"]}"##,
    );
    assert!(!out.contains_key("make_overhang_printable"));
    assert!(!out.contains_key("no_such_setting"));
    assert_eq!(one(&out, "wall_loops"), "3");
    assert!(report.dropped.contains("make_overhang_printable"));
    assert!(report.dropped.contains("no_such_setting"));
    assert!(report.notes[0].starts_with("project: 2 setting(s) left out: "));
    assert!(
        report
            .summary()
            .starts_with("2 setting(s) not supported by Bambu Studio left out, ")
    );
}

#[test]
fn enum_values_are_translated_or_left_out() {
    let (out, _, report) = project(
        r##"{"sparse_infill_pattern": "rectilinear", "top_surface_pattern": "lateral-lattice",
            "support_style": "organic", "ensure_vertical_shell_thickness": "ensure_all",
            "seam_position": "left", "brim_type": "painted", "bottom_surface_pattern": "monotonic",
            "nozzle_diameter": ["0.4"]}"##,
    );
    assert_eq!(one(&out, "sparse_infill_pattern"), "zig-zag");
    assert_eq!(one(&out, "top_surface_pattern"), "2dlattice");
    assert_eq!(one(&out, "support_style"), "tree_organic");
    assert_eq!(one(&out, "ensure_vertical_shell_thickness"), "enabled");
    assert_eq!(one(&out, "seam_position"), "aligned");
    assert_eq!(one(&out, "bottom_surface_pattern"), "monotonic");
    // Bambu has no painted brim: Bambu keeps its own.
    assert!(!out.contains_key("brim_type"));
    assert!(report.notes.iter().any(|note| {
        note.contains("left out brim_type: value \"painted\" does not exist in Bambu Studio")
    }));
    for (ours, theirs) in [
        ("none", "disabled"),
        ("ensure_critical_only", "partial"),
        ("ensure_moderate", "partial"),
    ] {
        assert_eq!(
            translate_enum_value("ensure_vertical_shell_thickness", ours),
            theirs
        );
    }
    assert_eq!(translate_enum_value("seam_position", "aligned_back"), "");
    assert_eq!(
        translate_enum_value("curr_bed_type", "Textured Cool Plate"),
        ""
    );
    assert_eq!(
        translate_enum_value("curr_bed_type", "Cool Plate"),
        "Cool Plate"
    );
}

#[test]
fn values_are_reshaped_to_bambus_types() {
    let (out, _, report) = project(
        r##"{"nozzle_diameter": ["0.4"], "travel_speed": "500", "outer_wall_speed": "200",
            "travel_speed_z": "12", "fan_max_speed": ["95.6"], "line_width": "110%",
            "seam_gap": "0.1", "wipe_speed": "250", "ironing_speed": ["30", "40"],
            "overhang_1_4_speed": "50%", "enable_support": "true", "flush_multiplier": "1"}"##,
    );
    // One value to a list (Bambu's per variant).
    assert_eq!(many(&out, "travel_speed_z"), ["12"]);
    // A fraction to a whole number.
    assert_eq!(many(&out, "fan_max_speed"), ["96"]);
    // "mm or %" to mm, of the nozzle (the reference's own case: 0.44).
    assert_eq!(one(&out, "line_width"), "0.44");
    // mm to Bambu's "%": of the nozzle, of the travel speed.
    assert_eq!(one(&out, "seam_gap"), "25%");
    assert_eq!(one(&out, "wipe_speed"), "50%");
    // A list to its first entry, said where they differed.
    assert_eq!(one(&out, "ironing_speed"), "30");
    assert!(report.notes.iter().any(|note| note
        == "project: ironing_speed: per-variant list collapsed to its first slot (Bambu has a single value)"));
    // Of the outer wall speed.
    assert_eq!(many(&out, "overhang_1_4_speed"), ["100"]);
    assert_eq!(one(&out, "enable_support"), "1");
    assert_eq!(many(&out, "flush_multiplier"), ["1"]);
    for key in [
        "line_width",
        "seam_gap",
        "wipe_speed",
        "fan_max_speed",
        "travel_speed_z",
    ] {
        assert!(report.converted.contains(key), "{key}");
    }
    // A "%" with nothing to be a percentage of is left out.
    let (out, _, report) = project(r##"{"line_width": "110%"}"##);
    assert!(!out.contains_key("line_width"));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.contains("relative value \"110%\" has nothing to resolve against"))
    );
    // "nil" only where Bambu lets a list be empty.
    let (out, _, report) = project(r##"{"nozzle_diameter": ["0.4"], "fan_max_speed": ["nil"]}"##);
    assert!(!out.contains_key("fan_max_speed"));
    assert!(report.dropped.contains("fan_max_speed"));
}

#[test]
fn one_extruder_is_one_variant_and_each_filament_a_slot() {
    let (out, ctx, _) = project(
        r##"{"nozzle_diameter": ["0.4"], "filament_colour": ["#FFFFFF", "#000000"],
            "nozzle_temperature": ["210", "230"], "filament_self_index": ["1", "1"],
            "outer_wall_speed": "200"}"##,
    );
    assert_eq!(ctx.printer.names, ["Direct Drive Standard"]);
    assert_eq!(
        many(&out, "printer_extruder_variant"),
        ["Direct Drive Standard"]
    );
    assert_eq!(many(&out, "printer_extruder_id"), ["1"]);
    assert_eq!(
        many(&out, "print_extruder_variant"),
        ["Direct Drive Standard"]
    );
    // Our own [1, 1] is not trusted: one slot per filament.
    assert_eq!(many(&out, "filament_self_index"), ["1", "2"]);
    assert_eq!(
        many(&out, "filament_extruder_variant"),
        ["Direct Drive Standard", "Direct Drive Standard"]
    );
    assert_eq!(many(&out, "nozzle_temperature"), ["210", "230"]);
    assert_eq!(many(&out, "outer_wall_speed"), ["200"]);
}

#[test]
fn two_extruders_are_laid_out_as_bambu_indexes_them() {
    let (out, ctx, report) = project(
        r##"{"nozzle_diameter": ["0.4", "0.4"], "extruder_type": ["Direct Drive"],
            "nozzle_volume_type": ["Standard", "High Flow"],
            "filament_colour": ["#FFFFFF", "#000000"], "filament_settings_id": ["A", "B"],
            "nozzle_temperature": ["210", "230"], "retraction_length": ["0.8", "1.2"],
            "machine_max_speed_x": ["500", "200", "400", "100"],
            "flush_volumes_matrix": ["0", "100", "200", "0"], "flush_multiplier": ["1"],
            "outer_wall_speed": "200"}"##,
    );
    assert_eq!(ctx.extruder_count, 2);
    // One variant per extruder, by its type and nozzle volume.
    assert_eq!(
        many(&out, "printer_extruder_variant"),
        ["Direct Drive Standard", "Direct Drive High Flow"]
    );
    assert_eq!(many(&out, "printer_extruder_id"), ["1", "2"]);
    // Each filament a slot for each distinct variant.
    assert_eq!(
        many(&out, "filament_extruder_variant"),
        [
            "Direct Drive Standard",
            "Direct Drive High Flow",
            "Direct Drive Standard",
            "Direct Drive High Flow"
        ]
    );
    assert_eq!(many(&out, "filament_self_index"), ["1", "1", "2", "2"]);
    assert_eq!(
        many(&out, "nozzle_temperature"),
        ["210", "210", "230", "230"]
    );
    // Per extruder, and (normal, silent) pairs per variant.
    assert_eq!(many(&out, "retraction_length"), ["0.8", "1.2"]);
    assert_eq!(
        many(&out, "machine_max_speed_x"),
        ["500", "200", "400", "100"]
    );
    // Per extruder in Bambu: one per extruder of each.
    assert_eq!(
        many(&out, "extruder_type"),
        ["Direct Drive", "Direct Drive"]
    );
    assert_eq!(many(&out, "flush_multiplier"), ["1", "1"]);
    assert_eq!(
        many(&out, "flush_volumes_matrix"),
        ["0", "100", "200", "0", "0", "100", "200", "0"]
    );
    assert!(report.converted.contains("flush_volumes_matrix"));
    // A preset of one filament is laid out for one filament.
    let mut preset_report = Report::default();
    let preset = convert(
        &settings(r##"{"nozzle_temperature": ["215"], "filament_settings_id": ["A"]}"##),
        &ctx,
        Scope::Filament,
        &mut preset_report,
        "preset A",
    );
    assert_eq!(many(&preset, "nozzle_temperature"), ["215", "215"]);
    assert_eq!(many(&preset, "filament_self_index"), ["1", "1"]);
    assert!(!preset.contains_key("printer_extruder_variant"));
    // An object's own filament option is one value.
    let object = convert(
        &settings(r##"{"nozzle_temperature": ["220"]}"##),
        &ctx,
        Scope::Object,
        &mut preset_report,
        "object Cube",
    );
    assert_eq!(many(&object, "nozzle_temperature"), ["220", "220"]);
    assert!(!object.contains_key("filament_self_index"));
}

#[test]
fn paint_that_follows_the_shell_goes_out_as_the_shells_depth() {
    let (out, ctx, report) = project(
        r##"{"nozzle_diameter": ["0.4"], "top_color_penetration_layers": "0",
            "bottom_color_penetration_layers": "3", "top_shell_layers": "4",
            "top_shell_thickness": "1", "layer_height": "0.2"}"##,
    );
    // max(4 layers, ceil(1 / 0.2) = 5).
    assert_eq!(one(&out, "top_color_penetration_layers"), "5");
    assert_eq!(one(&out, "bottom_color_penetration_layers"), "3");
    assert!(report.notes.iter().any(|note| note.contains(
        "top_color_penetration_layers 0 (follow the shell) written as 5, the depth the top shell settings give"
    )));
    // An object with shells of its own under a project that follows them
    // gets its own depth.
    let mut object_report = Report::default();
    let object = convert(
        &settings(r##"{"top_shell_layers": "7"}"##),
        &ctx,
        Scope::Object,
        &mut object_report,
        "object Cube",
    );
    assert_eq!(one(&object, "top_color_penetration_layers"), "7");
    assert!(!object.contains_key("bottom_color_penetration_layers"));
}

#[test]
fn bambus_own_constraints_are_kept() {
    let (out, _, report) = project(
        r##"{"nozzle_diameter": ["0.4"], "ooze_prevention": "1", "enable_prime_tower": "1",
            "extruder_nozzle_stats": [], "wipe_tower_interface_temp": "1",
            "wipe_tower_interface_run_in": "0"}"##,
    );
    assert!(!out.contains_key("ooze_prevention"));
    assert!(report.dropped.contains("ooze_prevention"));
    assert!(!out.contains_key("extruder_nozzle_stats"));
    assert_eq!(one(&out, "enable_tower_interface_features"), "1");
    assert!(report.notes.iter().any(|note| note.contains(
        "enable_tower_interface_features written as on although only wipe_tower_interface_temp is on"
    )));
}

#[test]
fn what_was_set_by_hand_is_named_in_bambus_words() {
    let (out, _, _) = project(
        r##"{"nozzle_diameter": ["0.4"],
            "different_settings_to_system": ["infill_anchor;make_overhang_printable;wall_loops", "", "only_one_wall_top"]}"##,
    );
    assert_eq!(
        many(&out, "different_settings_to_system"),
        ["sparse_infill_anchor;wall_loops", "", "top_one_wall_type"]
    );
}

#[test]
fn written_as_the_reference_writes_its_files() {
    let (out, _, _) = project(r##"{"nozzle_diameter": ["0.4"], "wall_loops": "3"}"##);
    let text = to_json(&out, "project_settings", "project", &export_version());
    assert!(text.starts_with(
        "{\n    \"filament_extruder_variant\": [\n        \"Direct Drive Standard\"\n    ],\n"
    ));
    assert!(text.contains("    \"version\": \"02.08.00.00\",\n"));
    assert!(text.contains("    \"wall_loops\": \"3\"\n"));
    assert!(text.ends_with("}\n"));
    let strings = |values: &[&str]| BambuValue {
        vector: true,
        strings: true,
        values: values.iter().map(|value| (*value).to_owned()).collect(),
    };
    assert_eq!(
        serialize(&strings(&["a", "b c", "d\"e"])),
        "a;\"b c\";\"d\\\"e\""
    );
    assert_eq!(serialize(&strings(&[""])), "\"\"");
    assert_eq!(serialize(&strings(&["a;b"])), "\"a;b\"");
    let text = BambuValue {
        vector: false,
        strings: true,
        values: vec!["G28\nG1 \"x\"".into()],
    };
    assert_eq!(serialize(&text), "G28\\nG1 \\\"x\\\"");
    assert_eq!(
        serialize(&BambuValue {
            vector: true,
            strings: false,
            values: vec!["1".into(), "2".into()],
        }),
        "1,2"
    );
    // An object's entries read back as their options have them.
    let read = object_settings([
        ("wall_loops", "3"),
        ("filament_colour", "#FF0000;\"a b\""),
        ("nozzle_temperature", "210,220"),
    ]);
    assert_eq!(read.text("wall_loops"), Some("3"));
    assert_eq!(read.texts("filament_colour").unwrap(), ["#FF0000", "a b"]);
    assert_eq!(read.texts("nozzle_temperature").unwrap(), ["210", "220"]);
}

#[test]
fn numbers_are_written_as_printf_g_writes_them() {
    for (value, text) in [
        (0.44, "0.44"),
        (0.1 * 3.0, "0.3"),
        (96.0, "96"),
        (25.000000000001, "25"),
        (1e-5, "1e-05"),
        (12345678901.0, "1.23456789e+10"),
        (-2.5, "-2.5"),
        (0.0, "0"),
        (100.0, "100"),
        (9999999999.5, "1e+10"),
    ] {
        assert_eq!(format_number(value), text, "{value}");
    }
}
