//! Settings no stage reads yet: each is said when it is set to a value
//! that would change the print, so that a profile which asks for one is
//! not followed in silence. An entry goes when the stage that should
//! follow it does.
//!
//! The table holds, for each setting, the value at which the reference
//! does nothing with it (its default, for nearly all; for some, only
//! while another setting is on), taken from the reference's
//! PrintConfig.cpp and the code that reads it. A setting that only
//! matters with another on is listed under that one (`covers`): the note
//! on the one that turns it on says it all. Settings are looked at here
//! unnoted ([`Settings::unnoted`]): saying that a setting is not followed
//! is no reading of it for the print, and what a slice reads is how the
//! desktop app tells which settings change nothing ([`crate::followed`]).

use edge_config::{Settings, Value};

/// The value at which the reference does nothing with a setting.
#[derive(Clone, Copy, Debug)]
enum Off {
    /// This value: numbers compared as numbers (with or without `%`), a
    /// flag's 0 as false; every entry of a list.
    Is(&'static str),
    /// No text: empty, or every entry empty or 0.
    Blank,
    /// Any number below this; the reference's default is shown.
    Below(f64, &'static str),
    /// Anything but the first; the second is shown as the value that
    /// does nothing.
    Unless(&'static str, &'static str),
}

/// A setting no stage reads, when it would change the print.
#[derive(Clone, Copy, Debug)]
struct Silent {
    key: &'static str,
    off: Off,
    /// Another setting it needs on (not at its value here) to change the
    /// print at all.
    with: Option<(&'static str, Off)>,
    /// What only changes the print with this one on, and so is said with it.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the list the guard test checks every setting against"
        )
    )]
    covers: &'static [&'static str],
    /// Whether it changes anything with one filament.
    many: Many,
}

/// How many filaments a setting needs to change anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Many {
    /// One is enough.
    One,
    /// More than one in the project: a filament it names past the
    /// project's is taken as the first (PrintObject.cpp
    /// `clamp_exturder_to_default`).
    InProject,
    /// More than one printing: it is about changing from one to another.
    Printing,
}

impl Silent {
    const fn many(self, many: Many) -> Self {
        Silent { many, ..self }
    }
}

const fn silent(key: &'static str, off: Off) -> Silent {
    Silent {
        key,
        off,
        with: None,
        covers: &[],
        many: Many::One,
    }
}

const fn gate(key: &'static str, off: Off, covers: &'static [&'static str]) -> Silent {
    Silent {
        key,
        off,
        with: None,
        covers,
        many: Many::One,
    }
}

const fn with(key: &'static str, off: Off, other: &'static str, other_off: Off) -> Silent {
    Silent {
        key,
        off,
        with: Some((other, other_off)),
        covers: &[],
        many: Many::One,
    }
}

use Off::{Below, Blank, Is, Unless};

/// What the reference does with each is in the comment over it (line
/// numbers in the pinned reference's `src/libslic3r`).
const SILENT: &[Silent] = &[
    // Speed and motion.
    // GCode.cpp l.7596, l.7635: a jerk per feature is written only with a
    // default jerk above 0.
    gate(
        "default_jerk",
        Is("0"),
        &[
            "outer_wall_jerk",
            "inner_wall_jerk",
            "infill_jerk",
            "top_surface_jerk",
            "initial_layer_jerk",
            "travel_jerk",
            "first_layer_travel_jerk",
        ],
    ),
    // GCode.cpp l.7600: M205 J, with Marlin 2 alone.
    with(
        "default_junction_deviation",
        Is("0"),
        "gcode_flavor",
        Unless("marlin2", "marlin"),
    ),
    // GCode.cpp l.11573: inner walls under an overhang slowed like it.
    gate(
        "overhang_shadow_inner_wall",
        Is("0"),
        &["overhang_shadow_distance", "overhang_shadow_speed_ratio"],
    ),
    // GCode.cpp l.11433: a ramp over more than one layer.
    silent("slow_down_layers", Below(2.0, "0")),
    // GCode.cpp l.10908: small support loops at their own speed.
    gate(
        "small_support_perimeter_threshold",
        Is("0"),
        &["small_support_perimeter_speed"],
    ),
    // LayerTimeSpeedSmoothingFilter.
    gate(
        "layer_time_speed_smoothing",
        Is("off"),
        &[
            "layer_time_speed_max_slowdown",
            "layer_time_speed_max_speedup",
            "layer_time_speed_max_time_increase",
            "layer_time_speed_max_variation",
            "layer_time_speed_slowdown_scope",
        ],
    ),
    // GCode.cpp l.9971: park and pause for the minimum layer time.
    gate(
        "min_layer_time_pause",
        Is("0"),
        &["min_layer_time_pause_park_x", "min_layer_time_pause_park_y"],
    ),
    // Extrusion.
    // PerimeterGenerator.hpp l.149: surfaces and walls over support,
    // printed with their own speed and flow (GCode.cpp l.11261, l.11371).
    gate(
        "over_support_surfaces",
        Is("0"),
        &["over_support_speed", "over_support_flow"],
    ),
    // Absolute E (M82) when off.
    silent("use_relative_e_distances", Is("1")),
    // GCode.cpp l.11720: with pressure advance on.
    Silent {
        key: "adaptive_pressure_advance",
        off: Is("0"),
        with: Some(("enable_pressure_advance", Is("0"))),
        covers: &[
            "adaptive_pressure_advance_model",
            "adaptive_pressure_advance_bridges",
            "adaptive_pressure_advance_overhangs",
        ],
        many: Many::One,
    },
    // PressureEqualizer.
    gate(
        "max_volumetric_extrusion_rate_slope",
        Is("0"),
        &[
            "max_volumetric_extrusion_rate_slope_segment_length",
            "extrusion_rate_smoothing_external_perimeter_only",
        ],
    ),
    // Fan.
    // GCodeWriter.cpp l.1449: a floor under any fan speed but 0.
    silent("part_cooling_fan_min_pwm", Is("0")),
    // CoolingBuffer.cpp l.936: a fan speed of its own when 0 or more.
    silent("support_material_interface_fan_speed", Below(0.0, "-1")),
    // Temperature: GCode.cpp l.4541, M141 / M191 with a chamber temperature.
    with(
        "activate_chamber_temp_control",
        Is("0"),
        "chamber_temperature",
        Is("0"),
    ),
    // Height: GCodeWriter adds it to every Z (GCode.cpp l.3742).
    silent("z_offset", Is("0")),
    silent("filament_z_offset", Is("0")),
    // Geometry.
    // PrintObjectSlice.cpp l.319.
    silent("enable_order_independent_overlap_carving", Is("0")),
    // PrintApply.cpp l.1476: the model scaled in XY, and in Z.
    silent("filament_shrink", Is("100%")),
    silent("filament_shrinkage_compensation_z", Is("100%")),
    // PrintObjectSlice.cpp l.198.
    silent("slicing_mode", Is("regular")),
    // FDMHollowing.cpp.
    gate("hollow_interior", Is("0"), &["hollow_shell_thickness"]),
    // Tool changes.
    // ToolOrdering.cpp l.1617, GCode.cpp l.493: flushing for "Fast"; ours
    // follows "Saving" alone.
    gate(
        "prime_volume_mode",
        Unless("Fast", "Default"),
        &["flush_multiplier_fast"],
    )
    .many(Many::Printing),
    // GCodeWriter.cpp l.705, GCode.cpp l.13182: tool changes written as
    // comments for a person to make.
    silent("manual_filament_change", Is("0")).many(Many::Printing),
    // ToolOrdering.cpp l.1404: every filament of the project loaded.
    silent("preload_all_filaments", Is("0")).many(Many::InProject),
    // PrintRegion.cpp l.58: walls and fill in a filament of their own.
    // An object that names its own filament sets these over the plate's
    // (PrintObject.cpp `apply_to_print_region_config`); said all the same,
    // as a model that names none prints them in it.
    silent("wall_filament", Is("1")).many(Many::InProject),
    silent("outer_wall_filament", Is("0")).many(Many::InProject),
    silent("sparse_infill_filament", Is("1")).many(Many::InProject),
    silent("solid_infill_filament", Is("1")).many(Many::InProject),
    // Mixed (virtual) filaments: PrintApply.cpp l.1808-1831.
    gate(
        "mixed_filament_definitions",
        Blank,
        &[
            "mixed_color_layer_height_a",
            "mixed_color_layer_height_b",
            "mixed_filament_advanced_dithering",
            "mixed_filament_component_bias_enabled",
            "mixed_filament_gradient_mode",
            "mixed_filament_height_lower_bound",
            "mixed_filament_height_upper_bound",
            "mixed_filament_pointillism_line_gap",
            "mixed_filament_pointillism_pixel_size",
            "mixed_filament_region_collapse",
            "mixed_filament_surface_indentation",
        ],
    ),
    // EdgeSlicer's per-nozzle processes and nozzle rack (NozzleProcess.cpp,
    // ToolSlots.cpp); not in edge-config's lists of settings.
    gate(
        "nozzle_process_presets",
        Blank,
        &["nozzle_process_missing", "nozzle_process_edits"],
    ),
    gate(
        "nozzle_slot_process_presets",
        Blank,
        &["nozzle_slot_process_missing"],
    ),
    silent("nozzle_region_values", Blank),
    silent("process_extruder_values", Blank),
    silent("process_nozzle_line_widths", Blank),
    silent("line_width_nozzle_diameter", Blank),
    silent("print_extruder_nozzle_diameter", Blank),
    silent("nozzle_diameter_override", Blank),
    silent("nozzle_layer_height", Blank),
    silent("nozzle_filament_process_slot", Blank),
    silent("nozzle_filament_tool", Blank),
    silent("filament_nozzle_variants", Blank),
    silent("tool_slot_model", Blank),
    silent("support_filament_nozzle_mapping", Blank),
    silent("extruder_rack_nozzle_diameter", Blank),
    silent("extruder_rack_nozzle_override", Blank),
    silent("filament_rack_nozzle_diameter", Blank),
    silent("filament_rack_nozzle_slot", Blank),
    silent("filament_side_pin", Blank),
    // Support.
    // SupportParameters.hpp l.237: woven interface layers.
    gate(
        "support_interface_interlock",
        Is("0"),
        &[
            "support_interface_interlock_beam_lines",
            "support_interface_interlock_layers",
        ],
    ),
    // Stabilizers.cpp: read here only to keep vase mode and arranging
    // right; no stabilizer is built.
    gate(
        "stabilizer_supports",
        Is("off"),
        &[
            "stabilizer_brace_max_span",
            "stabilizer_brace_max_unbraced",
            "stabilizer_bracing",
            "stabilizer_column_length",
            "stabilizer_column_min_height",
            "stabilizer_column_shape",
            "stabilizer_column_width",
            "stabilizer_infill_density",
            "stabilizer_infill_pattern",
            "stabilizer_max_island_width",
            "stabilizer_pillar_base_diameter",
            "stabilizer_pillar_diameter",
            "stabilizer_points_per_ring",
            "stabilizer_ring_spacing",
            "stabilizer_tip_diameter",
            "stabilizer_tip_gap",
            "stabilizer_wall_loops",
        ],
    ),
    // Brim and adhesion.
    // Fill.
    // Fill.cpp l.1527: set from 0 degrees up.
    silent("bottom_layer_direction", Below(0.0, "-1")),
    // Fill.cpp l.1494.
    silent("bottom_surface_fill_order", Is("default")),
    // PrintObject.cpp l.5727.
    silent("extra_solid_infills", Blank),
    // Fill.cpp l.1523.
    silent("sparse_infill_smooth_factor", Is("0%")),
    // Fill.cpp l.1439.
    silent("undertop_surface_pattern", Is("default")),
    // PrintObject.cpp l.3126.
    silent("bottom_surface_density", Is("100%")),
    silent("top_surface_density", Is("100%")),
    // PrintObject.cpp l.2289: with top surface expansion on.
    with(
        "top_surface_expansion_margin",
        Is("0"),
        "top_surface_expansion",
        Is("0"),
    ),
    with(
        "top_surface_expansion_direction",
        Is("inward_and_outward"),
        "top_surface_expansion",
        Is("0"),
    ),
    // PrintObject.cpp l.5938.
    gate(
        "infill_combination",
        Is("0"),
        &["infill_combination_max_layer_height"],
    ),
    // PrintObject.cpp l.554.
    silent("separated_infills", Is("0")),
    // Walls: PerimeterGenerator.cpp l.346.
    silent("offset_layers", Is("0")),
    // Skirt: GCode.cpp l.7129.
    silent("single_loop_draft_shield", Is("0")),
    // Order of objects: GCode.cpp l.3711.
    silent("print_order", Is("default")),
    // G-code of the profile's own: GCode.cpp l.11741, l.4345.
    silent("change_extrusion_role_gcode", Blank),
    silent("filament_change_extrusion_role_gcode", Blank),
    silent("process_change_extrusion_role_gcode", Blank),
    silent("file_start_gcode", Blank),
    // What the file carries besides moves: GCodeWriter l.688, GCode.cpp
    // l.3429.
    silent("disable_m73", Is("0")),
    silent("gcode_skip_config_block", Is("0")),
];

/// Two numbers or two texts alike, as the reference would read them.
fn alike(value: &str, off: &str) -> bool {
    let (value, off) = (value.trim(), off.trim());
    let number = |text: &str| text.strip_suffix('%').unwrap_or(text).parse::<f64>();
    value == off
        || matches!((number(value), number(off)), (Ok(x), Ok(y)) if x == y)
        || matches!(
            (value, off),
            ("1" | "true", "1" | "true") | ("0" | "false", "0" | "false")
        )
}

impl Off {
    /// Whether one entry is at the value that does nothing.
    fn holds(self, entry: &str) -> bool {
        match self {
            Is(off) => alike(entry, off),
            Blank => matches!(entry.trim(), "" | "0" | "\"\""),
            Below(limit, _) => entry
                .trim()
                .strip_suffix('%')
                .unwrap_or(entry.trim())
                .parse::<f64>()
                .is_ok_and(|number| number < limit),
            Unless(on, _) => entry.trim() != on,
        }
    }

    /// The value that does nothing, as the note gives it.
    fn shown(self) -> &'static str {
        match self {
            Is(off) | Below(_, off) | Unless(_, off) => off,
            Blank => "",
        }
    }

    /// The first entry of `value` that would change the print, if any.
    fn first_on(self, value: &Value) -> Option<&str> {
        match value {
            Value::One(text) => (!self.holds(text)).then_some(text.as_str()),
            Value::Many(texts) => texts
                .iter()
                .map(String::as_str)
                .find(|text| !self.holds(text)),
        }
    }
}

/// A value as a note gives it: its first line, cut short when long.
fn quoted(value: &str) -> String {
    const LONG: usize = 40;
    let line = value.lines().next().unwrap_or("");
    let cut: String = line.chars().take(LONG).collect();
    if cut.len() < value.len() {
        format!("{cut}…")
    } else {
        cut
    }
}

/// The note for `key` set to `value`, where the print is made as with
/// `off`: one sentence for every setting.
fn note(key: &str, value: &str, off: Off) -> String {
    format!(
        "{key} \"{}\": not followed; the print is made as if it were \"{}\"",
        quoted(value),
        off.shown()
    )
}

/// What `settings` ask for that nothing follows, for a print in
/// `printing` filaments.
pub(crate) fn notes(settings: &Settings, printing: usize) -> Vec<String> {
    let in_project = (settings.unnoted("filament_diameter")).map_or(1, |value| match value {
        Value::One(_) => 1,
        Value::Many(entries) => entries.len(),
    });
    SILENT
        .iter()
        .filter(|silent| match silent.many {
            Many::One => true,
            Many::InProject => in_project > 1,
            Many::Printing => printing > 1,
        })
        .filter(|silent| {
            silent.with.is_none_or(|(other, off)| {
                (settings.unnoted(other)).is_some_and(|value| off.first_on(value).is_some())
            })
        })
        .filter_map(|silent| {
            let value = settings.unnoted(silent.key)?;
            let on = silent.off.first_on(value)?;
            Some(note(silent.key, on, silent.off))
        })
        .collect()
}

/// The table's entry that heads `key`'s note, if it has one.
fn heading(key: &str) -> Option<&'static Silent> {
    SILENT.iter().find(|silent| silent.key == key)
}

/// Whether `value` set on an object, a part or a modifier for `key` is the
/// value at which the reference does nothing with it, where nothing reads
/// `key`: the print there is then what the reference makes, and nothing
/// is said.
pub(crate) fn off_where_set(key: &str, value: &str) -> bool {
    heading(key).is_some_and(|silent| silent.off.holds(value))
}

/// The note for `key` set on `on` (an object, a part, a modifier, a
/// range) where nothing reads it, in the same sentence as the plate's.
pub(crate) fn note_where_set(key: &str, on: &str) -> Option<String> {
    let silent = heading(key)?;
    Some(format!(
        "{key}: set on {on}, not followed; the print is made as if it were \"{}\"",
        silent.off.shown()
    ))
}

/// Every setting the table says something of: those that head a note and
/// those said with them. For the test that keeps every setting either
/// read, said or listed as changing nothing.
#[cfg(test)]
pub(crate) fn said() -> impl Iterator<Item = &'static str> {
    SILENT
        .iter()
        .flat_map(|silent| std::iter::once(silent.key).chain(silent.covers.iter().copied()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeSet,
        path::{Path, PathBuf},
    };

    use super::*;

    #[test]
    fn nothing_is_said_of_settings_left_off() {
        assert!(notes(&Settings::default(), 1).is_empty());
        // The reference's own defaults are each at the value that does
        // nothing.
        let said = notes(edge_config::reference_defaults(), 1);
        assert!(said.is_empty(), "{said:?}");
        // Turned off in other words, nothing either.
        let off = Settings::from_json(
            r#"{"z_offset": "0.0", "default_jerk": ["0"], "filament_shrink": ["100", "100%"],
                "slow_down_layers": "1", "support_material_interface_fan_speed": ["-1", "-5"],
                "bottom_layer_direction": "-3", "use_relative_e_distances": "true",
                "file_start_gcode": "", "mixed_filament_definitions": "\"\"",
                "default_junction_deviation": "0.02", "gcode_flavor": "klipper",
                "adaptive_pressure_advance": ["1"], "enable_pressure_advance": ["0"],
                "prime_volume_mode": "Saving"}"#,
        )
        .unwrap();
        let said = notes(&off, 1);
        assert!(said.is_empty(), "{said:?}");
    }

    #[test]
    fn the_older_tree_styles_brim_is_not_said_for_organic_trees() {
        // The reference reads them only in the older styles' generator,
        // which support_style's own note covers (measured: the same
        // G-code for tree(auto) with either set otherwise).
        let set = Settings::from_json(
            r#"{"tree_support_auto_brim": "0", "tree_support_brim_width": "10"}"#,
        )
        .unwrap();
        assert!(notes(&set, 1).is_empty());
    }

    #[test]
    fn settings_nothing_reads_are_said_when_they_change_the_print() {
        let on = Settings::from_json(
            r#"{"z_offset": "0.2", "filament_shrink": ["100%", "98%"], "slow_down_layers": "3",
                "default_junction_deviation": "0.02", "gcode_flavor": "marlin2",
                "adaptive_pressure_advance": ["1"], "enable_pressure_advance": ["1"],
                "prime_volume_mode": "Fast", "stabilizer_supports": "auto",
                "file_start_gcode": "M117 first line\nM117 second line"}"#,
        )
        .unwrap();
        let said = notes(&on, 2);
        assert_eq!(
            said,
            [
                "default_junction_deviation \"0.02\": not followed; the print is made as if it were \"0\"",
                "slow_down_layers \"3\": not followed; the print is made as if it were \"0\"",
                "adaptive_pressure_advance \"1\": not followed; the print is made as if it were \"0\"",
                "z_offset \"0.2\": not followed; the print is made as if it were \"0\"",
                "filament_shrink \"98%\": not followed; the print is made as if it were \"100%\"",
                "prime_volume_mode \"Fast\": not followed; the print is made as if it were \"Default\"",
                "stabilizer_supports \"auto\": not followed; the print is made as if it were \"off\"",
                "file_start_gcode \"M117 first line…\": not followed; the print is made as if it were \"\"",
            ]
            .map(String::from),
        );
        // Each starts with its setting's name, as the desktop app finds
        // a setting's notes by.
        let followed = crate::Followed {
            notes: said,
            ..Default::default()
        };
        assert_eq!(followed.notes_on("slow_down").count(), 0);
        assert_eq!(followed.notes_on("z_offset").count(), 1);
        // Looking does not count as reading: what a slice reads is how the
        // desktop app tells a setting that changes nothing.
        let reads = edge_config::Reads::default();
        let mut noted = on.clone();
        noted.noted_by(&reads);
        let _ = notes(&noted, 1);
        assert!(reads.keys().is_empty());
    }

    #[test]
    fn what_is_about_more_filaments_than_one_waits_for_them() {
        let one = Settings::from_json(
            r#"{"manual_filament_change": "1", "wall_filament": "2",
                "filament_diameter": ["1.75"]}"#,
        )
        .unwrap();
        assert!(notes(&one, 1).is_empty());
        // Two printing: tool changes are written.
        let said = notes(&one, 2);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("manual_filament_change \"1\""));
        // Two in the project: walls may go to the second though only the
        // first is printing.
        let two =
            Settings::from_json(r#"{"wall_filament": "2", "filament_diameter": ["1.75", "1.75"]}"#)
                .unwrap();
        let said = notes(&two, 1);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(said[0].starts_with("wall_filament \"2\""));
    }

    #[test]
    fn where_set_on_an_object_the_value_that_does_nothing_is_not_said() {
        assert!(off_where_set("over_support_surfaces", "0"));
        assert!(!off_where_set("over_support_surfaces", "1"));
        // What is said with another setting is not the table's to weigh.
        assert!(!off_where_set("over_support_speed", "0"));
        assert_eq!(
            note_where_set("over_support_surfaces", "an object").as_deref(),
            Some(
                "over_support_surfaces: set on an object, not followed; the print is made as if it were \"0\""
            )
        );
        assert_eq!(note_where_set("wall_loops", "an object"), None);
    }

    /// Settings no stage names that change nothing in the print, or are
    /// said by another setting's note, by the classes of the settings
    /// audit of 2026-11-16 (each judged from the reference's code).
    mod allowed {
        /// Said by another setting's note, or refused: lightning, lockedzag, brim ears,
        /// raft, older tree styles, support filaments, interlocking beams, two-nozzle
        /// Bambu printers, adaptive bed mesh.
        pub const NOTED: &[&str] = &[
            "adaptive_bed_mesh_margin",
            "adaptive_infill_gradient",
            "adaptive_infill_interior_density",
            "adaptive_infill_mode",
            "adaptive_layer_height",
            "bed_mesh_max",
            "bed_mesh_min",
            "bed_mesh_probe_distance",
            "brim_ears_detection_length",
            "brim_ears_max_angle",
            "brim_ears_outer_only",
            "dithering_local_z_direct_multicolor",
            "dithering_local_z_infill",
            "dithering_local_z_whole_objects",
            "dithering_step_painted_zones_only",
            "dithering_z_step_size",
            "enable_pre_heating",
            "extruder_max_nozzle_count",
            "extruder_printable_area",
            "extruder_printable_height",
            "filament_flush_temp",
            "filament_flush_temp_fast",
            "filament_flush_volumetric_speed",
            "filament_map_mode",
            "filament_nozzle_map",
            "filament_pre_cooling_temperature",
            "filament_pre_cooling_temperature_nc",
            "filament_preheat_temperature_delta",
            "filament_retract_length_nc",
            "filament_volume_map",
            "group_algo_with_time",
            "hotend_cooling_rate",
            "hotend_heating_rate",
            "infill_instead_top_bottom_surfaces",
            "infill_lock_depth",
            "interlocking_beam_layer_count",
            "interlocking_beam_width",
            "interlocking_boundary_avoidance",
            "interlocking_depth",
            "interlocking_orientation",
            "lightning_overhang_angle",
            "lightning_prune_angle",
            "lightning_straightening_angle",
            "locked_skeleton_infill_pattern",
            "locked_skin_infill_pattern",
            "long_retractions_when_ec",
            "machine_switch_extruder_time",
            "master_extruder_id",
            "nozzle_temperature_range_high",
            "override_paint",
            "physical_extruder_map",
            "raft_contact_distance",
            "raft_expansion",
            "retraction_distances_when_ec",
            "skeleton_infill_density",
            "skeleton_infill_line_width",
            "skin_infill_density",
            "skin_infill_depth",
            "skin_infill_line_width",
            "support_filament_matching",
            "support_group",
            "support_interface_not_for_body",
            "support_ironing_direction",
            "support_ironing_flow",
            "support_ironing_inset",
            "support_ironing_pattern",
            "support_ironing_spacing",
            "support_ironing_speed",
            // Read only by the older tree styles' generator
            // (TreeSupport.cpp l.2500, `draw_circles`), which organic
            // trees never reach (`TreeSupport::generate` hands them to
            // TreeSupport3D first): said by support_style's note.
            "tree_support_auto_brim",
            "tree_support_branch_angle",
            "tree_support_branch_diameter",
            "tree_support_branch_distance",
            "tree_support_brim_width",
        ];
        /// Changing nothing in the G-code or the slice: the GUI, the network, preset
        /// choice, showcase rendering, file names, cost and time estimates, warnings.
        pub const GUI: &[&str] = &[
            "bbl_calib_mark_logo",
            "bbl_use_printhost",
            "bed_custom_model",
            "bed_custom_texture",
            "compatible_printers_condition",
            "compatible_prints",
            "compatible_prints_condition",
            "default_filament_colour",
            "delta_temperature",
            "enable_filament_dynamic_map",
            "extruder_ams_count",
            "extruder_colour",
            "extruder_nozzle_count",
            "extruder_nozzle_stats",
            "extruder_variant_list",
            "filament_colour_angle",
            "filament_colour_mode",
            "filament_colour_split",
            "filament_colour_twist",
            "filament_cost",
            "filament_extruder_variant",
            "filament_flow_step_size",
            "filament_flow_support",
            "filament_glow",
            "filament_glow_colour",
            "filament_glow_intensity",
            "filament_gradient_dwell",
            "filament_gradient_transition",
            "filament_map_2",
            "filament_notes",
            "filament_self_index",
            "filename_format",
            "fins_extrude_safe_temp",
            "first_layer_sequence_choice",
            "flashforge_serial_number",
            "gcode_add_line_number",
            "grab_length",
            "has_filament_switcher",
            "host_type",
            "image_fill_detail",
            "image_fill_params",
            "machine_prepare_time",
            "notes",
            "nozzle_flush_dataset",
            "nozzle_hrc",
            "nozzle_temperature_range_low",
            "nozzle_type",
            "other_layers_sequence_choice",
            "pellet_flow_coefficient",
            "pellet_modded_printer",
            "post_process",
            "preferred_orientation",
            "preheat_steps",
            "preset_name",
            "preset_names",
            "print_extruder_id",
            "print_extruder_variant",
            "printer_extruder_id",
            "printer_extruder_variant",
            "printer_flow_support",
            "printer_technology",
            "printhost_authorization_type",
            "printhost_ssl_ignore_revoke",
            "project_schema_version",
            "required_nozzle_HRC",
            "silent_mode",
            "spaghetti_detector",
            "support_air_filtration",
            "support_chamber_temp_control",
            "support_multi_bed_types",
            "thumb0",
            "thumb1",
            "thumbnails",
            "thumbnails_format",
            "time_cost",
            "unload_filaments_at_end",
            "upward_compatible_machine",
        ];
        /// The prime / wipe tower and SEMM ramming: on hold, listed only.
        pub const TOWER: &[&str] = &[
            "cooling_tube_length",
            "cooling_tube_retraction",
            "extra_loading_move",
            "filament_cooling_final_speed",
            "filament_cooling_initial_speed",
            "filament_cooling_moves",
            "filament_loading_speed",
            "filament_loading_speed_start",
            "filament_stamping_distance",
            "filament_stamping_loading_speed",
            "filament_toolchange_delay",
            "filament_tower_interface_pre_extrusion_dist",
            "filament_tower_interface_pre_extrusion_length",
            "filament_tower_interface_print_temp",
            "filament_unloading_speed",
            "filament_unloading_speed_start",
            "high_current_on_filament_swap",
            "local_z_wipe_tower_purge_lines",
            "parking_pos_retraction",
            "prime_tower_wipe_distance",
            "prime_tower_wipe_speed",
            "printer_notes",
            "single_extruder_multi_material_priming",
            "wait_for_temp_on_wipe_tower",
            "wipe_tower_cone_angle",
            "wipe_tower_interface_trigger",
            "z_hop_when_prime",
        ];
        /// Read by nothing in the reference either.
        pub const DEAD: &[&str] = &[
            "brim_ears",
            "filament_grouping_mode",
            "has_scarf_joint_seam",
            "min_layer_time_pause_park",
            "mixed_filament_auto_gradient_choice",
            "mixed_filament_auto_gradient_physical_count",
            "tree_support_with_infill",
            "wiping_volumes_extruders",
        ];
        /// Set only as a placeholder by the reference, which this engine gives under
        /// every setting's name.
        pub const PLACEHOLDER: &[&str] = &["chamber_minimal_temperature"];
        /// Being made to be followed on branches of their own; each comes
        /// off here when it lands.
        pub const IN_HAND: &[&str] = &[];
        /// Only matter with toolchange_ordering "cyclic", which has a
        /// note of its own (pipeline.rs).
        pub const SAID_BY_ANOTHER: &[&str] =
            &["toolchange_cyclic_order", "toolchange_cyclic_first_layer"];
    }

    /// Read by a stage and said where only partly followed: kept in the
    /// table though a stage names them.
    const PARTLY: [&str; 2] = ["stabilizer_supports", "prime_volume_mode"];

    /// The workspace's crates.
    fn crates() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("crates/")
            .to_path_buf()
    }

    /// Every `"name"` in `text`, as a setting's name would be written.
    fn literals(text: &str, into: &mut BTreeSet<String>) {
        let bytes = text.as_bytes();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'"' {
                let start = at + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                {
                    end += 1;
                }
                if end > start
                    && bytes[start].is_ascii_alphabetic()
                    && bytes.get(end) == Some(&b'"')
                {
                    into.insert(text[start..end].to_owned());
                    at = end + 1;
                    continue;
                }
            }
            at += 1;
        }
    }

    /// A file's text without its tests: from a `#[cfg(test)]` over a
    /// `mod` to the end.
    fn without_tests(text: &str) -> &str {
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if let Some(rest) = line.strip_prefix("#[cfg(test)]") {
                let after = &text[offset + line.len() - rest.len()..];
                let trimmed = after.trim_start();
                let gap = &after[..after.len() - trimmed.len()];
                if gap.contains('\n') && trimmed.starts_with("mod ") {
                    return &text[..offset];
                }
            }
            offset += line.len();
        }
        text
    }

    fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if path.is_dir() {
                if !matches!(name.to_str(), Some("tests" | "benches" | "target")) {
                    sources(&path, out);
                }
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push(path);
            }
        }
    }

    /// The settings named by a stage: as a literal in a crate's own code
    /// (not its tests), outside the crates and files that only write,
    /// list or carry settings, or built from one at run time.
    fn named_by_a_stage() -> BTreeSet<String> {
        const CARRIERS: [&str; 4] = ["edge-config", "edge-parity", "edge-3mf", "edge-cli"];
        const LISTS: [&str; 6] = [
            "calib",
            "regions",
            "followed",
            "unfollowed",
            "sliced_file",
            "model",
        ];
        let mut named = BTreeSet::new();
        let (mut gcode, mut time) = (BTreeSet::new(), BTreeSet::new());
        let mut entries: Vec<_> = std::fs::read_dir(crates()).unwrap().flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if CARRIERS.contains(&name.as_str()) {
                continue;
            }
            let mut files = Vec::new();
            sources(&entry.path(), &mut files);
            for file in files {
                let text = std::fs::read_to_string(&file).unwrap();
                let mut found = BTreeSet::new();
                literals(without_tests(&text), &mut found);
                let listing = name == "edge-engine"
                    && file
                        .parent()
                        .and_then(Path::file_name)
                        .is_some_and(|dir| dir == "src")
                    && file
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .is_some_and(|stem| LISTS.contains(&stem));
                match name.as_str() {
                    "edge-gcode" => gcode.extend(found.iter().cloned()),
                    "edge-time" => time.extend(found.iter().cloned()),
                    _ => {}
                }
                if !listing {
                    named.extend(found);
                }
            }
        }
        // Names built at run time: `filament_{retraction}` and
        // `{bed}_initial_layer` in the writer, the machine limits by axis.
        for stem in &gcode {
            named.insert(format!("filament_{stem}"));
            named.insert(format!("{stem}_initial_layer"));
        }
        for axis in ["x", "y", "z", "e"] {
            named.insert(format!("machine_max_jerk_{axis}"));
            for stem in &time {
                named.insert(format!("{stem}_{axis}"));
            }
        }
        named
    }

    /// Every setting the reference has, as edge-config lists them: its
    /// defaults, with those it lists without one. (The Bambu Studio keys
    /// edge-config also knows, for "Export Bambu 3MF", are not the
    /// reference's settings: it acts on none of them.)
    fn listed() -> BTreeSet<String> {
        let defaults = crates()
            .join("edge-config")
            .join("src")
            .join("reference_defaults.txt");
        std::fs::read_to_string(defaults)
            .unwrap()
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                let line = line.strip_prefix('?').unwrap_or(line);
                line.split_once(" =")
                    .map_or(line, |(key, _)| key)
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn no_setting_is_left_out_in_silence() {
        let named = named_by_a_stage();
        let said: BTreeSet<&str> = said().collect();
        let allowed: BTreeSet<&str> = [
            allowed::NOTED,
            allowed::GUI,
            allowed::TOWER,
            allowed::DEAD,
            allowed::PLACEHOLDER,
            allowed::IN_HAND,
            allowed::SAID_BY_ANOTHER,
        ]
        .concat()
        .into_iter()
        .collect();
        // A setting is read by a stage, said when set, or listed as one
        // that changes nothing (or is said otherwise).
        let silent: Vec<String> = listed()
            .into_iter()
            .filter(|key| {
                !named.contains(key)
                    && !said.contains(key.as_str())
                    && !allowed.contains(key.as_str())
            })
            .collect();
        assert!(
            silent.is_empty(),
            "settings no stage reads, with no note and not listed as changing nothing \
             (add each to unfollowed.rs `SILENT` with the value that does nothing, or to \
             `allowed` with why): {silent:?}"
        );
        // Said and listed both: one or the other.
        let both: Vec<&&str> = said.intersection(&allowed).collect();
        assert!(both.is_empty(), "{both:?}");
        // Said, though a stage reads it now: its entry goes.
        let stale: Vec<&&str> = said
            .iter()
            .filter(|key| named.contains(**key) && !PARTLY.contains(*key))
            .collect();
        assert!(
            stale.is_empty(),
            "read by a stage now; take them out of unfollowed.rs `SILENT`: {stale:?}"
        );
    }
}
