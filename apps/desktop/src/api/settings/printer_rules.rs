//! Which printer options are shown and which can be set: a port of the
//! reference's `TabPrinter::toggle_options` (src/slic3r/GUI/Tab.cpp), in
//! its order, every page's rules, and an extruder page's for each
//! extruder (its options named `key#n`, n from 0). With it the one rule a
//! group of `build_unregular_pages` keeps for itself: "Enable pressure
//! change when wiping" turns its value on and off.
//!
//! Left out, each where it stands: the values the reference changes by
//! itself while it toggles (`manual_filament_change` turned off without
//! single extruder multi-material, and the question that turns wiping off
//! under firmware retraction); the input shaper's list being cut to what
//! the G-code flavor takes (`update_input_shaper_menu`).

use edge_config::Settings;

use super::rules::Toggles;

struct Printer<'a>(&'a Settings);

impl Printer<'_> {
    /// Entry `index` of a list, or a setting's one value.
    fn at(&self, key: &str, index: usize) -> &str {
        self.0
            .texts(key)
            .and_then(|texts| texts.get(index).or(texts.last()))
            .map_or("", |text| text.trim())
    }

    fn text(&self, key: &str) -> &str {
        self.at(key, 0)
    }

    fn on(&self, key: &str) -> bool {
        matches!(self.text(key), "1" | "true")
    }

    fn on_at(&self, key: &str, index: usize) -> bool {
        matches!(self.at(key, index), "1" | "true")
    }

    fn number_at(&self, key: &str, index: usize) -> f64 {
        self.at(key, index)
            .trim_end_matches('%')
            .trim()
            .parse()
            .unwrap_or(0.0)
    }
}

/// How many extruders the settings describe: an entry of
/// `nozzle_diameter` each.
pub(super) fn extruders(settled: &Settings) -> usize {
    settled
        .texts("nozzle_diameter")
        .map_or(1, |sizes| sizes.len().max(1))
}

/// The rules, for the settings a slice would be given (`settled`).
pub(super) fn toggles(settled: &Settings) -> Toggles {
    let mut complete = edge_config::reference_defaults().clone();
    complete.overlay(settled);
    let p = Printer(&complete);
    let mut t = Toggles::default();
    let is_bbl_printer = p.text("printer_model").starts_with("Bambu Lab");
    let flavor = p.text("gcode_flavor");
    let extruders_count = extruders(&complete);

    // "Basic information"
    for key in [
        "scan_first_layer",
        "bbl_calib_mark_logo",
        "bbl_use_printhost",
    ] {
        t.line(key, is_bbl_printer);
    }
    for key in [
        "use_firmware_retraction",
        "use_relative_e_distances",
        "support_multi_bed_types",
        "pellet_modded_printer",
        "bed_mesh_max",
        "bed_mesh_min",
        "bed_mesh_probe_distance",
        "adaptive_bed_mesh_margin",
        "thumbnails",
    ] {
        t.line(key, !is_bbl_printer);
    }
    t.line(
        "enable_power_loss_recovery",
        is_bbl_printer || flavor == "marlin2",
    );
    // PrintConfig.cpp `is_snapmaker_toolchanger`.
    let toolchanger = p.text("printer_model").to_lowercase().contains("snapmaker")
        && extruders_count >= 2
        && !p.on("single_extruder_multi_material");
    t.line("unload_filaments_at_end", toolchanger);

    // "Multimaterial"
    t.line("wipe_tower_type", !is_bbl_printer);
    let supports_wipe_tower_2 = !is_bbl_printer && p.text("wipe_tower_type") == "type2";
    for key in [
        "enable_filament_ramming",
        "cooling_tube_retraction",
        "cooling_tube_length",
        "parking_pos_retraction",
        "extra_loading_move",
        "high_current_on_filament_swap",
    ] {
        t.field(key, supports_wipe_tower_2);
    }
    let semm = p.on("single_extruder_multi_material");
    t.field("extruders_count", !semm);
    t.field("manual_filament_change", semm);
    t.field("purge_in_prime_tower", semm && supports_wipe_tower_2);
    t.field(
        "tool_change_on_wipe_tower",
        !semm && supports_wipe_tower_2 && extruders_count > 1,
    );
    t.field(
        "wait_for_temp_on_wipe_tower",
        !semm && supports_wipe_tower_2 && extruders_count > 1,
    );
    // The group's own rule (`build_unregular_pages`, "Wipe tower").
    t.field(
        "ramming_pressure_advance_value",
        p.on("enable_change_pressure_when_wiping"),
    );

    // "Extruder n"
    let use_firmware_retraction = p.on("use_firmware_retraction");
    let have_multiple_extruders = true;
    for i in 0..extruders_count {
        let mut field = |key: &str, enabled: bool| t.field(&format!("{key}#{i}"), enabled);
        let have_retract_length = p.number_at("retraction_length", i) > 0.0;
        // The reference greys out "retract_length", a name no option has
        // (the option is `retraction_length`), so the length stays open
        // under firmware retraction; the firmware deciding the length is
        // plainly why the line is there, and the length is greyed here.
        // The same misspelt name stands in the list below, of what is off
        // without retraction; the length is not put there, or a length of
        // 0 could never be set to more.
        field("retraction_length", !use_firmware_retraction);
        field(
            "retraction_minimum_travel",
            have_retract_length || use_firmware_retraction,
        );
        let retraction = have_retract_length || use_firmware_retraction;
        for key in ["z_hop", "retract_when_changing_layer"] {
            field(key, retraction);
        }
        let lifts = retraction && p.number_at("z_hop", i) > 0.0;
        for key in [
            "retract_lift_above",
            "retract_lift_below",
            "retract_lift_enforce",
        ] {
            field(key, lifts);
        }
        for key in [
            "retraction_speed",
            "deretraction_speed",
            "retract_before_wipe",
            "retract_after_wipe",
            "retract_restart_extra",
            "wipe_distance",
        ] {
            field(key, retraction && !use_firmware_retraction);
        }
        let wipe = retraction && p.on_at("wipe", i);
        // Before and after shares add up to at most 100% (Orca #11015).
        let full = |key: &str| (p.number_at(key, i) - 100.0).abs() < 1e-9;
        field("retract_before_wipe", wipe && !full("retract_after_wipe"));
        field("retract_after_wipe", wipe && !full("retract_before_wipe"));
        field("wipe_distance", wipe);
        field("retract_length_toolchange", have_multiple_extruders);
        let toolchange_retraction = p.number_at("retract_length_toolchange", i) > 0.0;
        field(
            "retract_restart_extra_toolchange",
            have_multiple_extruders && toolchange_retraction,
        );
        field(
            "long_retractions_when_cut",
            !use_firmware_retraction && p.number_at("enable_long_retraction_when_cut", 0) != 0.0,
        );
        field("travel_slope", p.at("z_hop_types", i) != "Normal Lift");
        // The reference hides the first extruder's line on every page
        // ("retraction_distances_when_cut#0"); each extruder's own here.
        t.line(
            &format!("retraction_distances_when_cut#{i}"),
            p.on_at("long_retractions_when_cut", i),
        );
    }

    // "Motion ability"
    let marlin_legacy = flavor == "marlin";
    let marlin_firmware = flavor == "marlin2";
    let klipper = flavor == "klipper";
    let reprap_firmware = flavor == "reprapfirmware";
    t.field(
        "machine_max_acceleration_travel",
        !marlin_legacy && !klipper,
    );
    t.line(
        "machine_max_acceleration_travel",
        !marlin_legacy && !klipper,
    );
    t.field("machine_max_junction_deviation", marlin_firmware);
    t.line("machine_max_junction_deviation", marlin_firmware);
    let enable_jerk = !marlin_firmware
        || complete
            .texts("machine_max_junction_deviation")
            .is_none_or(|values| {
                values
                    .iter()
                    .all(|value| value.trim().parse::<f64>().ok() == Some(0.0))
            });
    for key in [
        "machine_max_jerk_x",
        "machine_max_jerk_y",
        "machine_max_jerk_z",
        "machine_max_jerk_e",
    ] {
        t.field(key, enable_jerk);
    }
    t.field(
        "emit_machine_limits_to_gcode",
        marlin_legacy || marlin_firmware || reprap_firmware,
    );
    let resonance_avoidance = p.on("resonance_avoidance");
    t.field("min_resonance_avoidance_speed", resonance_avoidance);
    t.field("max_resonance_avoidance_speed", resonance_avoidance);
    let input_shaping_compatible =
        (marlin_firmware || reprap_firmware || klipper) && !is_bbl_printer;
    for key in [
        "input_shaping_emit",
        "input_shaping_type",
        "input_shaping_freq_x",
        "input_shaping_freq_y",
        "input_shaping_damp_x",
        "input_shaping_damp_y",
    ] {
        t.line(key, input_shaping_compatible);
    }
    if input_shaping_compatible {
        let emit = klipper || p.on("emit_machine_limits_to_gcode");
        t.field("input_shaping_emit", emit);
        let shaping = emit && p.on("input_shaping_emit");
        t.field("input_shaping_type", shaping);
        t.field("input_shaping_freq_x", shaping);
        t.field("input_shaping_freq_y", shaping && !reprap_firmware);
        t.field("input_shaping_damp_x", shaping);
        t.field("input_shaping_damp_y", shaping && !reprap_firmware);
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(json: &str) -> Toggles {
        toggles(&Settings::from_json(json).unwrap())
    }

    #[test]
    fn each_extruder_has_its_own_rules() {
        let t = with(
            r#"{"nozzle_diameter": ["0.4", "0.4"], "retraction_length": ["0.8", "0"],
                "z_hop": ["0.4", "0.4"], "wipe": ["1", "1"]}"#,
        );
        assert!(!t.off("z_hop#0") && t.off("z_hop#1"));
        assert!(!t.off("retract_lift_above#0") && t.off("retract_lift_above#1"));
        assert!(!t.off("wipe_distance#0") && t.off("wipe_distance#1"));
    }

    #[test]
    fn firmware_retraction_greys_the_length() {
        let t = with(r#"{"nozzle_diameter": ["0.4"], "use_firmware_retraction": "1"}"#);
        assert!(t.off("retraction_length#0"));
        assert!(t.off("retraction_speed#0"));
        assert!(!t.off("z_hop#0"));
    }

    #[test]
    fn the_flavor_decides_the_motion_limits() {
        let klipper = with(r#"{"gcode_flavor": "klipper"}"#);
        assert!(klipper.hidden("machine_max_acceleration_travel"));
        assert!(klipper.hidden("machine_max_junction_deviation"));
        assert!(!klipper.hidden("input_shaping_type") && !klipper.off("input_shaping_emit"));
        let marlin2 =
            with(r#"{"gcode_flavor": "marlin2", "machine_max_junction_deviation": ["0.02", "0"]}"#);
        assert!(!marlin2.hidden("machine_max_junction_deviation"));
        assert!(marlin2.off("machine_max_jerk_x"));
        assert!(!marlin2.hidden("enable_power_loss_recovery"));
    }

    #[test]
    fn a_snapmaker_toolchanger_unloads_at_the_end() {
        let u1 = with(
            r#"{"printer_model": "Snapmaker U1", "single_extruder_multi_material": "0", "nozzle_diameter": ["0.4", "0.4", "0.4", "0.4"]}"#,
        );
        assert!(!u1.hidden("unload_filaments_at_end"));
        let one = with(r#"{"printer_model": "Snapmaker U1", "nozzle_diameter": ["0.4"]}"#);
        assert!(one.hidden("unload_filaments_at_end"));
    }
}
