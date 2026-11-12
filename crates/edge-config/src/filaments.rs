//! A project's settings laid out for another list of filaments: one taken
//! away, one added, the list put in another order.
//!
//! The reference keeps a project's filaments as a list of filament presets
//! (`PresetBundle::filament_presets`) and the project's own lists beside it
//! (`project_config`: `filament_colour` and the rest), and builds the flat
//! settings a print reads from them (PresetBundle.cpp `full_fff_config`):
//! every filament option a list with an entry for each filament. A filament
//! added takes the preset of the last (`set_num_filaments`, `resize(n,
//! filament_presets.back())`), so its entries are the last filament's; one
//! taken away takes its entries with it (`update_num_filaments`). Here a
//! project's own settings, which hold that flat list already, are laid out
//! anew by saying, for each filament of the new list, which filament of the
//! old one it is ([`reslot`]).

use crate::{Settings, Value};

/// The options a filament preset holds (Preset.cpp `s_Preset_filament_options`),
/// each a list with an entry for each filament in a project's settings.
/// Left out: what names the presets a filament suits (`compatible_prints`,
/// `compatible_printers` and their conditions) and `inherits`, which a
/// project holds otherwise.
const FILAMENT_OPTIONS: &[&str] = &[
    "filament_extruder_variant",
    "filament_self_index",
    "filament_nozzle_variants",
    "default_filament_colour",
    "required_nozzle_HRC",
    "filament_diameter",
    "pellet_flow_coefficient",
    "filament_type",
    "filament_soluble",
    "filament_is_support",
    "filament_max_volumetric_speed",
    "filament_flow_ratio",
    "filament_density",
    "filament_cost",
    "filament_z_offset",
    "filament_minimal_purge_on_wipe_tower",
    "nozzle_temperature",
    "nozzle_temperature_initial_layer",
    "cool_plate_temp",
    "textured_cool_plate_temp",
    "eng_plate_temp",
    "hot_plate_temp",
    "textured_plate_temp",
    "cool_plate_temp_initial_layer",
    "textured_cool_plate_temp_initial_layer",
    "eng_plate_temp_initial_layer",
    "hot_plate_temp_initial_layer",
    "textured_plate_temp_initial_layer",
    "supertack_plate_temp_initial_layer",
    "supertack_plate_temp",
    "graphic_effect_plate_temp",
    "graphic_effect_plate_temp_initial_layer",
    "temperature_vitrification",
    "fins_extrude_safe_temp",
    "filament_is_high_temperature",
    "reduce_fan_stop_start_freq",
    "dont_slow_down_outer_wall",
    "slow_down_for_layer_cooling",
    "fan_min_speed",
    "fan_max_speed",
    "enable_overhang_bridge_fan",
    "overhang_fan_speed",
    "overhang_fan_threshold",
    "close_fan_the_first_x_layers",
    "full_fan_speed_layer",
    "initial_layer_fan_speed",
    "fan_cooling_layer_time",
    "slow_down_layer_time",
    "slow_down_min_speed",
    "filament_start_gcode",
    "filament_end_gcode",
    "filament_change_extrusion_role_gcode",
    "activate_air_filtration",
    "activate_air_filtration_during_print",
    "activate_air_filtration_on_completion",
    "during_print_exhaust_fan_speed",
    "complete_print_exhaust_fan_speed",
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
    "filament_retract_before_wipe",
    "filament_retract_after_wipe",
    "filament_retract_length_toolchange",
    "filament_retract_restart_extra_toolchange",
    "filament_vendor",
    "filament_wipe_distance",
    "additional_cooling_fan_speed",
    "nozzle_temperature_range_low",
    "nozzle_temperature_range_high",
    "enable_pressure_advance",
    "pressure_advance",
    "adaptive_pressure_advance",
    "adaptive_pressure_advance_model",
    "adaptive_pressure_advance_overhangs",
    "adaptive_pressure_advance_bridges",
    "chamber_temperature",
    "chamber_minimal_temperature",
    "filament_shrink",
    "filament_shrinkage_compensation_z",
    "support_material_interface_fan_speed",
    "internal_bridge_fan_speed",
    "filament_notes",
    "ironing_fan_speed",
    "filament_ironing_flow",
    "filament_ironing_spacing",
    "filament_ironing_inset",
    "filament_ironing_speed",
    "filament_loading_speed",
    "filament_loading_speed_start",
    "filament_unloading_speed",
    "filament_unloading_speed_start",
    "filament_toolchange_delay",
    "filament_cooling_moves",
    "filament_stamping_loading_speed",
    "filament_stamping_distance",
    "filament_cooling_initial_speed",
    "filament_cooling_final_speed",
    "filament_ramming_parameters",
    "filament_multitool_ramming",
    "filament_multitool_ramming_volume",
    "filament_multitool_ramming_flow",
    "activate_chamber_temp_control",
    "filament_long_retractions_when_cut",
    "filament_retraction_distances_when_cut",
    "idle_temperature",
    "long_retractions_when_ec",
    "retraction_distances_when_ec",
    "filament_tower_ironing_area",
    "filament_tower_interface_print_temp",
    "filament_tower_interface_pre_extrusion_dist",
    "filament_tower_interface_pre_extrusion_length",
    "circle_compensation_speed",
    "counter_coef_1",
    "counter_coef_2",
    "counter_coef_3",
    "hole_coef_1",
    "hole_coef_2",
    "hole_coef_3",
    "counter_limit_min",
    "counter_limit_max",
    "hole_limit_min",
    "hole_limit_max",
    "diameter_limit",
    "filament_pre_cooling_temperature_nc",
    "filament_retract_length_nc",
    "filament_pre_cooling_temperature",
    "filament_preheat_temperature_delta",
    "filament_flush_volumetric_speed",
    "filament_flush_temp",
    "filament_flush_temp_fast",
    "filament_cooling_before_tower",
    "filament_flow_support",
    // What a project holds for each filament beside its preset's options
    // (PresetBundle.cpp `full_fff_config`, `project_config`): its name,
    // colour, flow, the nozzle it is put on.
    "filament_settings_id",
    "filament_ids",
    "filament_colour",
    "filament_multi_colors",
    "filament_colour_type",
    "filament_map",
    "filament_nozzle_map",
    "filament_volume_type",
    "filament_nozzle_assignment",
    "filament_rack_nozzle_diameter",
    "filament_rack_nozzle_slot",
    "filament_side_pin",
    "filament_flow_step_size",
    "filament_adhesiveness_category",
    "filament_printable",
    "filament_scarf_seam_type",
    "filament_scarf_height",
    "filament_scarf_gap",
    "filament_scarf_length",
];

/// The lists with an entry for the process, one for each filament and one
/// for the printer (PresetBundle.cpp `full_fff_config`).
const GROUPS: [&str; 2] = ["different_settings_to_system", "inherits_group"];

/// Process settings that name a filament (MixedFilamentConfigRemap.hpp
/// `mixed_filament_feature_keys`): 0 is "default".
pub const FILAMENT_NUMBERS: [&str; 5] = [
    "wall_filament",
    "sparse_infill_filament",
    "solid_infill_filament",
    "support_filament",
    "support_interface_filament",
];

/// Whether a setting is one of a filament's, a list with an entry for each.
pub fn is_per_filament(key: &str) -> bool {
    FILAMENT_OPTIONS.contains(&key)
}

/// How many filaments a project's settings describe: the entries of
/// `filament_settings_id`, else of `filament_colour`, else of the
/// filaments' diameters; one when they say nothing.
pub fn count(settings: &Settings) -> usize {
    [
        "filament_settings_id",
        "filament_colour",
        "filament_diameter",
    ]
    .iter()
    .find_map(
        |key| match settings.iter().find(|(name, _)| name == key)?.1 {
            Value::Many(items) if !items.is_empty() => Some(items.len()),
            Value::Many(_) => None,
            Value::One(_) => Some(1),
        },
    )
    .unwrap_or(1)
}

/// Settings laid out for another list of filaments: filament n + 1 of the
/// new list is filament `sources[n]` + 1 of the old one, and takes its
/// entry of every filament's list. A list that is not one entry for each
/// old filament is left as it is. A project the reference's application
/// saved with each filament's flow variants side by side
/// (`filament_flow_step_size`) has each filament's whole run of them moved.
/// The process settings that name a filament are renumbered by `number`
/// (old number from 1 to new, 0 for none: "default").
pub fn reslot(settings: &Settings, sources: &[usize], number: impl Fn(usize) -> usize) -> Settings {
    let old = count(settings);
    let steps: Option<Vec<usize>> = settings
        .iter()
        .find(|(key, _)| *key == "filament_flow_step_size")
        .map(|(_, value)| match value {
            Value::One(text) => vec![text.trim().parse::<usize>().unwrap_or(1).max(1)],
            Value::Many(texts) => texts
                .iter()
                .map(|text| text.trim().parse::<usize>().unwrap_or(1).max(1))
                .collect(),
        })
        .filter(|steps| steps.len() == old && steps.iter().any(|step| *step != 1));
    let starts: Vec<usize> = steps
        .as_ref()
        .map(|steps| {
            steps
                .iter()
                .scan(0, |start, step| {
                    let this = *start;
                    *start += step;
                    Some(this)
                })
                .collect()
        })
        .unwrap_or_default();
    let packed_len = steps.as_ref().map(|steps| steps.iter().sum::<usize>());
    let mut out = Settings::default();
    for (key, value) in settings.iter() {
        let laid = match value {
            Value::Many(items) if is_per_filament(key) && items.len() == old => Value::Many(
                sources
                    .iter()
                    .map(|source| items[*source].clone())
                    .collect(),
            ),
            // Each filament's run of flow variants.
            Value::Many(items)
                if is_per_filament(key)
                    && Some(items.len()) == packed_len
                    && key != "filament_flow_step_size" =>
            {
                let steps = steps.as_ref().expect("packed");
                Value::Many(
                    sources
                        .iter()
                        .flat_map(|source| {
                            items[starts[*source]..starts[*source] + steps[*source]]
                                .iter()
                                .cloned()
                        })
                        .collect(),
                )
            }
            Value::Many(items) if GROUPS.contains(&key) && items.len() == old + 2 => {
                let mut groups = vec![items[0].clone()];
                groups.extend(sources.iter().map(|source| items[source + 1].clone()));
                groups.push(items[old + 1].clone());
                Value::Many(groups)
            }
            Value::One(text) if FILAMENT_NUMBERS.contains(&key) => {
                match text.trim().parse::<usize>() {
                    Ok(at) => Value::One(number(at).to_string()),
                    Err(_) => value.clone(),
                }
            }
            // A list of one, where a project of one filament may write a
            // filament's option as a single value.
            Value::One(text) if is_per_filament(key) && old == 1 && sources.len() != 1 => {
                Value::Many(vec![text.clone(); sources.len()])
            }
            _ => value.clone(),
        };
        out.insert(key, laid);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(json: &str) -> Settings {
        Settings::from_json(json).unwrap()
    }

    #[test]
    fn a_filament_added_takes_the_last_ones_entries_and_one_taken_away_takes_its_own() {
        let three = settings(
            r##"{"filament_settings_id": ["A", "B", "C"], "nozzle_temperature": ["200", "210", "220"],
                "filament_colour": ["#111111", "#222222", "#333333"], "nozzle_diameter": ["0.4", "0.4", "0.4"],
                "different_settings_to_system": ["p", "a", "b", "c", "m"], "support_filament": "3",
                "wall_filament": "2"}"##,
        );
        assert_eq!(count(&three), 3);
        // Filament 2 taken away: 3 becomes 2, what named 2 names none.
        let renumber = |at: usize| match at {
            2 => 0,
            3 => 2,
            other => other,
        };
        let two = reslot(&three, &[0, 2], renumber);
        assert_eq!(two.texts("filament_settings_id").unwrap(), ["A", "C"]);
        assert_eq!(two.texts("nozzle_temperature").unwrap(), ["200", "220"]);
        assert_eq!(
            two.texts("filament_colour").unwrap(),
            ["#111111", "#333333"]
        );
        // The printer's lists are the printer's.
        assert_eq!(two.texts("nozzle_diameter").unwrap(), ["0.4", "0.4", "0.4"]);
        assert_eq!(
            two.texts("different_settings_to_system").unwrap(),
            ["p", "a", "c", "m"]
        );
        assert_eq!(two.text("support_filament"), Some("2"));
        assert_eq!(two.text("wall_filament"), Some("0"));
        // One added: the last again.
        let four = reslot(&three, &[0, 1, 2, 2], |at| at);
        assert_eq!(
            four.texts("nozzle_temperature").unwrap(),
            ["200", "210", "220", "220"]
        );
        assert_eq!(count(&four), 4);
    }

    #[test]
    fn a_filaments_run_of_flow_variants_goes_with_it() {
        let packed = settings(
            r#"{"filament_settings_id": ["A", "B"], "filament_flow_step_size": ["2", "1"],
                "nozzle_temperature": ["220", "230", "240"],
                "filament_flow_support": ["standard", "high_flow", "standard"]}"#,
        );
        let swapped = reslot(&packed, &[1, 0, 0], |at| at);
        assert_eq!(
            swapped.texts("nozzle_temperature").unwrap(),
            ["240", "220", "230", "220", "230"]
        );
        assert_eq!(
            swapped.texts("filament_flow_step_size").unwrap(),
            ["1", "2", "2"]
        );
        assert_eq!(
            swapped.texts("filament_settings_id").unwrap(),
            ["B", "A", "A"]
        );
    }
}
