//! Which filament options are shown and which can be set: a port of the
//! reference's `TabFilament::toggle_options` and of the overrides page's
//! own rules, `TabFilament::update_filament_overrides_page`
//! (src/slic3r/GUI/Tab.cpp), in their order. The reference runs each
//! page's rules when the page is shown; here they all run, each option
//! being on one page only.
//!
//! The reference reads the filament preset being edited, one filament;
//! here that filament's entry of each list is read, and the printer's
//! settings as the slice would get them.
//!
//! Left out: the value the reference changes by itself while it toggles
//! (`initial_layer_fan_speed` set to -1 while "No cooling for the first"
//! is on); the size view's locks (`m_size_locked`), which belong to a
//! per-nozzle view this app has not got.

use std::collections::BTreeMap;

use edge_config::Settings;

use super::rules::Toggles;

/// One filament's entry of the settings a slice would get, and the
/// printer's settings among them.
struct Filament<'a> {
    settings: &'a Settings,
    index: usize,
}

impl Filament<'_> {
    /// The filament's own entry of a list, or a setting's one value.
    fn own(&self, key: &str) -> &str {
        self.settings
            .texts(key)
            .and_then(|texts| texts.get(self.index).or(texts.last()))
            .map_or("", |text| text.trim())
    }

    /// A printer's (or the project's) setting: of a list, the first.
    fn printer(&self, key: &str) -> &str {
        self.settings
            .texts(key)
            .and_then(|texts| texts.first())
            .map_or("", |text| text.trim())
    }

    fn on(&self, key: &str) -> bool {
        matches!(self.own(key), "1" | "true")
    }

    fn printer_on(&self, key: &str) -> bool {
        matches!(self.printer(key), "1" | "true")
    }

    fn number(&self, key: &str) -> f64 {
        self.own(key)
            .trim_end_matches('%')
            .trim()
            .parse()
            .unwrap_or(0.0)
    }
}

/// The plates' lines of "Bed temperature", by the bed type they are for
/// (PrintConfig.hpp `BedType`, by the names the settings give them).
const PLATES: [(&str, [&str; 2]); 7] = [
    (
        "Supertack Plate",
        ["supertack_plate_temp_initial_layer", "supertack_plate_temp"],
    ),
    (
        "Cool Plate",
        ["cool_plate_temp_initial_layer", "cool_plate_temp"],
    ),
    (
        "Textured Cool Plate",
        [
            "textured_cool_plate_temp_initial_layer",
            "textured_cool_plate_temp",
        ],
    ),
    (
        "Engineering Plate",
        ["eng_plate_temp_initial_layer", "eng_plate_temp"],
    ),
    (
        "High Temp Plate",
        ["hot_plate_temp_initial_layer", "hot_plate_temp"],
    ),
    (
        "Textured PEI Plate",
        ["textured_plate_temp_initial_layer", "textured_plate_temp"],
    ),
    (
        "Graphic Effect Plate",
        [
            "graphic_effect_plate_temp_initial_layer",
            "graphic_effect_plate_temp",
        ],
    ),
];

/// The retraction overrides, in the reference's order
/// (`update_filament_overrides_page`).
const RETRACTION: [&str; 19] = [
    "filament_retraction_length",
    "filament_z_hop",
    "filament_z_hop_types",
    "filament_retract_lift_above",
    "filament_retract_lift_below",
    "filament_retract_lift_enforce",
    "filament_retraction_speed",
    "filament_deretraction_speed",
    "filament_retract_restart_extra",
    "filament_retraction_minimum_travel",
    "filament_retract_when_changing_layer",
    "filament_wipe",
    "filament_wipe_distance",
    "filament_retract_before_wipe",
    "filament_retract_after_wipe",
    "filament_long_retractions_when_cut",
    "filament_retraction_distances_when_cut",
    "filament_retract_length_toolchange",
    "filament_retract_restart_extra_toolchange",
];

/// The rules for filament `index` (from 0), with the settings a slice
/// would be given (`settled`, over the reference's defaults). Also the
/// labels the rules give lines, by the first option on the line.
pub(super) fn toggles(settled: &Settings, index: usize) -> (Toggles, BTreeMap<String, String>) {
    let mut complete = edge_config::reference_defaults().clone();
    complete.overlay(settled);
    let f = Filament {
        settings: &complete,
        index,
    };
    let mut t = Toggles::default();
    let mut labels = BTreeMap::new();
    let is_bbl_printer = f.printer("printer_model").starts_with("Bambu Lab");

    // "Cooling"
    let has_enable_overhang_bridge_fan = f.on("enable_overhang_bridge_fan");
    for key in [
        "overhang_fan_speed",
        "overhang_fan_threshold",
        "internal_bridge_fan_speed",
    ] {
        t.field(key, has_enable_overhang_bridge_fan);
    }
    t.field(
        "additional_cooling_fan_speed",
        f.printer_on("auxiliary_fan"),
    );
    let activate_air_filtration = f.on("activate_air_filtration");
    t.field(
        "activate_air_filtration_during_print",
        activate_air_filtration,
    );
    t.field(
        "activate_air_filtration_on_completion",
        activate_air_filtration,
    );
    t.field(
        "during_print_exhaust_fan_speed",
        activate_air_filtration && f.on("activate_air_filtration_during_print"),
    );
    t.field(
        "complete_print_exhaust_fan_speed",
        activate_air_filtration && f.on("activate_air_filtration_on_completion"),
    );
    t.field(
        "dont_slow_down_outer_wall",
        f.on("slow_down_for_layer_cooling"),
    );
    t.line(
        "initial_layer_fan_speed",
        f.number("close_fan_the_first_x_layers") <= 0.0,
    );

    // "Filament"
    let pa = f.on("enable_pressure_advance");
    t.field("pressure_advance", pa);
    let support_multi_bed_types = is_bbl_printer || f.printer_on("support_multi_bed_types");
    let model = f.printer("printer_model");
    let name = f.printer("printer_settings_id");
    let is_snapmaker_u1 = name.contains("Snapmaker U1")
        || (model.to_lowercase().contains("snapmaker") && model.to_lowercase().contains("u1"));
    labels.insert(
        "hot_plate_temp_initial_layer".to_owned(),
        if is_snapmaker_u1 {
            "Smooth PEI Plate"
        } else {
            "Smooth PEI Plate / High Temp Plate"
        }
        .to_owned(),
    );
    labels.insert(
        "supertack_plate_temp_initial_layer".to_owned(),
        if is_snapmaker_u1 {
            "Cool Steel Plate"
        } else {
            "Cool Plate (SuperTack)"
        }
        .to_owned(),
    );
    let plate_shown = |plate: &str| -> bool {
        if is_snapmaker_u1 && !support_multi_bed_types {
            // A U1's own plates, and the Cool Plate.
            matches!(
                plate,
                "Cool Plate" | "High Temp Plate" | "Textured PEI Plate" | "Graphic Effect Plate"
            )
        } else if support_multi_bed_types {
            plate != "Graphic Effect Plate" || is_snapmaker_u1
        } else {
            // The printer's own plate only (`get_default_bed_type`); a
            // printer without a choice is on it.
            f.printer("curr_bed_type") == plate
        }
    };
    for (plate, keys) in PLATES {
        t.lines(&keys, plate_shown(plate));
    }
    t.field("adaptive_pressure_advance", pa);
    t.field("adaptive_pressure_advance_overhangs", pa);
    let has_adaptive_pa = f.on("adaptive_pressure_advance");
    for key in [
        "adaptive_pressure_advance_overhangs",
        "adaptive_pressure_advance_model",
        "adaptive_pressure_advance_bridges",
    ] {
        t.line(key, has_adaptive_pa && pa);
    }
    let is_pellet_printer = f.printer_on("pellet_modded_printer");
    t.line("pellet_flow_coefficient", is_pellet_printer);
    t.line("filament_diameter", !is_pellet_printer);
    // The reference hides "chamber_temperatures", a name no option has
    // (the option is `chamber_temperature`), so the line never goes; the
    // chamber's line is plainly what is meant, and is hidden here.
    t.line(
        "chamber_temperature",
        f.printer_on("support_chamber_temp_control"),
    );

    // "Setting Overrides": a retraction override can be turned on only
    // while the filament retracts at all (its own length unset, or more
    // than 0); the long retractions only where the printer leaves them to
    // the filament (`enable_long_retraction_when_cut` 2, EnableFilament).
    let length = f.own("filament_retraction_length");
    let have_retract_length =
        length == "nil" || length.is_empty() || f.number("filament_retraction_length") > 0.0;
    let machine_enabled = f.printer("enable_long_retraction_when_cut") == "2";
    for key in RETRACTION {
        let can = key == "filament_retraction_length" || have_retract_length;
        match key {
            "filament_long_retractions_when_cut" => {
                t.line(key, machine_enabled);
                t.field(key, can && machine_enabled);
            }
            "filament_retraction_distances_when_cut" => {
                let filament_enabled = f.own("filament_long_retractions_when_cut") == "1";
                t.line(key, filament_enabled && machine_enabled);
                t.field(key, can && filament_enabled && machine_enabled);
            }
            _ => t.field(key, can),
        }
    }

    // "Multimaterial"
    for key in [
        "filament_minimal_purge_on_wipe_tower",
        "filament_loading_speed_start",
        "filament_loading_speed",
        "filament_unloading_speed_start",
        "filament_unloading_speed",
        "filament_toolchange_delay",
        "filament_cooling_moves",
        "filament_cooling_initial_speed",
        "filament_cooling_final_speed",
    ] {
        t.field(key, !is_bbl_printer);
    }
    let multitool_ramming = f.on("filament_multitool_ramming");
    t.field("filament_multitool_ramming_volume", multitool_ramming);
    t.field("filament_multitool_ramming_flow", multitool_ramming);
    // The project's prime volume mode (PrintConfig.hpp `PrimeVolumeMode`).
    let fast_purge = f.printer("prime_volume_mode") == "Fast";
    t.line("filament_flush_temp", !fast_purge);
    t.line("filament_flush_temp_fast", fast_purge);
    (t, labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(json: &str, index: usize) -> (Toggles, BTreeMap<String, String>) {
        toggles(&Settings::from_json(json).unwrap(), index)
    }

    #[test]
    fn a_filaments_own_entry_decides_its_rules() {
        let json = r#"{"enable_overhang_bridge_fan": ["1", "0"], "enable_pressure_advance": ["0", "1"],
                       "printer_model": "Snapmaker U1", "curr_bed_type": "Textured PEI Plate"}"#;
        let (first, labels) = with(json, 0);
        assert!(!first.off("overhang_fan_speed"));
        assert!(first.off("pressure_advance"));
        let (second, _) = with(json, 1);
        assert!(second.off("overhang_fan_speed"));
        assert!(!second.off("pressure_advance"));
        // A U1 shows its own plates and names them its way.
        assert!(first.hidden("eng_plate_temp") && !first.hidden("textured_plate_temp"));
        assert_eq!(
            labels["supertack_plate_temp_initial_layer"],
            "Cool Steel Plate"
        );
        assert_eq!(labels["hot_plate_temp_initial_layer"], "Smooth PEI Plate");
    }

    #[test]
    fn a_printer_without_a_choice_of_plates_shows_its_own() {
        let (t, labels) = with(
            r#"{"printer_model": "Generic", "curr_bed_type": "Cool Plate", "support_multi_bed_types": "0"}"#,
            0,
        );
        assert!(!t.hidden("cool_plate_temp") && t.hidden("hot_plate_temp"));
        assert_eq!(
            labels["supertack_plate_temp_initial_layer"],
            "Cool Plate (SuperTack)"
        );
    }

    #[test]
    fn an_override_needs_the_filament_to_retract() {
        let (on, _) = with(r#"{"filament_retraction_length": ["nil"]}"#, 0);
        assert!(!on.off("filament_z_hop"));
        let (off, _) = with(r#"{"filament_retraction_length": ["0"]}"#, 0);
        assert!(off.off("filament_z_hop") && !off.off("filament_retraction_length"));
        // The long retractions are the printer's to give.
        assert!(off.hidden("filament_long_retractions_when_cut"));
        let (given, _) = with(
            r#"{"enable_long_retraction_when_cut": "2", "filament_long_retractions_when_cut": ["1"]}"#,
            0,
        );
        assert!(!given.hidden("filament_retraction_distances_when_cut"));
    }

    #[test]
    fn the_chamber_line_follows_the_printer() {
        let (without, _) = with(r#"{"support_chamber_temp_control": "0"}"#, 0);
        assert!(without.hidden("chamber_temperature"));
        let (with_control, _) = with(r#"{"support_chamber_temp_control": "1"}"#, 0);
        assert!(!with_control.hidden("chamber_temperature"));
    }
}
