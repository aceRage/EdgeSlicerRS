//! The names Bambu Studio 2.x machine templates use that are worked out
//! rather than set, as the reference gives them to the start G-code and
//! keeps them for the rest of the print (GCode.cpp `_do_export`, the
//! "Ultra: publish BambuStudio 2.x placeholder vars" block, and
//! `bambu_flush_placeholders`). The Bambu Lab A2L, H2S and P2S start
//! G-codes cannot be filled without them.

use edge_template::{Value, Variables};

/// The flush placeholders: for each filament the volumetric speed it is
/// flushed at (`filament_flush_volumetric_speed`, its
/// `filament_max_volumetric_speed` where 0) and the temperature
/// (`filament_flush_temp`, or `filament_flush_temp_fast` with
/// `prime_volume_mode` fast; the top of its recommended range,
/// `nozzle_temperature_range_high`, where 0). The four lists are one
/// entry per filament here already, at each filament's flow variant
/// (`edge_config::variants`); an entry that is not a number (`nil`) reads
/// as the setting's default, 0, as in the reference's
/// `filament_variant_value`.
pub(crate) fn flush(variables: &mut Variables, filaments: usize) {
    let fast = variables
        .value("prime_volume_mode")
        .is_some_and(|mode| matches!(mode.to_string().trim(), "Fast" | "2"));
    // A filament's entry, the first past the list's end (`get_at`).
    let at = |variables: &Variables, name: &str, filament: usize| -> f64 {
        let value = match variables.list(name) {
            Some(list) => list.get(filament).or(list.first()).cloned(),
            None => variables.value(name),
        };
        value
            .and_then(|value| value.to_string().trim().parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
    };
    let mut speeds = Vec::with_capacity(filaments);
    let mut temperatures = Vec::with_capacity(filaments);
    for filament in 0..filaments {
        let mut speed = at(variables, "filament_flush_volumetric_speed", filament);
        if speed == 0.0 {
            speed = at(variables, "filament_max_volumetric_speed", filament);
        }
        speeds.push(Value::Float(speed));
        let key = if fast {
            "filament_flush_temp_fast"
        } else {
            "filament_flush_temp"
        };
        // An integer setting: a fraction is cut off, as the reference
        // reads `ConfigOptionInts`.
        let mut temperature = at(variables, key, filament) as i64;
        if temperature == 0 {
            temperature = at(variables, "nozzle_temperature_range_high", filament) as i64;
        }
        temperatures.push(Value::Int(temperature));
    }
    variables.set_list("flush_volumetric_speeds", speeds);
    variables.set_list("flush_temperatures", temperatures);
}

/// What the reference gives the templates of a printer of one nozzle for
/// the names Bambu Studio works out from its filament grouping and its own
/// checks (GCode.cpp `_do_export`). Several are fixed values there, which
/// are kept: `has_tpu_in_first_layer`, `cooling_filter_enabled` and
/// `hold_chamber_temp_for_flat_print` false and `max_print_z` 0 (so a
/// Bambu start G-code's check for obstacles under the bed, `G3811`, is
/// never written), and the prime tower interface and wipe-avoid ones off;
/// a note says so where a template uses one ([`FIXED`]). The timelapse
/// spot Bambu Studio picks (`has_timelapse_safe_pos`, `timelapse_pos_x`,
/// `timelapse_pos_y`) is not given, so a template that uses it is refused,
/// and `clear_to_x0` only printing layer by layer, where the A2L's
/// template does not need it.
pub(crate) fn single_nozzle(
    variables: &mut Variables,
    filaments: usize,
    first: usize,
    used: &[usize],
    by_layer: bool,
) {
    let nozzles = variables
        .list("nozzle_diameter")
        .map_or(1, <[Value]>::len)
        .max(1);
    let filaments = filaments.max(1);
    let first = first as i64;
    let int = Value::Int;
    if by_layer {
        variables.set("clear_to_x0", Value::Bool(true));
    }
    // Every filament on the one nozzle; with more, as the settings have it.
    if nozzles == 1 {
        variables.set_list("filament_map", vec![int(1); filaments]);
    }
    let vendors = variables.list("filament_vendor").unwrap_or(&[]);
    let all_bambu = used.iter().all(|filament| {
        vendors
            .get(*filament)
            .is_none_or(|vendor| vendor.to_string().contains("Bambu"))
    });
    variables.set("is_all_bbl_filament", Value::Bool(all_bambu));
    for name in FIXED
        .iter()
        .chain(["is_prime_tower_interface", "wipe_avoid_perimeter"].iter())
    {
        variables.set(*name, Value::Bool(false));
    }
    variables.set("max_print_z", Value::Float(0.0));
    variables.set("filament_tower_interface_purge_volume", Value::Float(0.0));
    variables.set("filament_tower_interface_print_temp", int(0));
    variables.set("wipe_avoid_pos_x", Value::Float(0.0));
    // Per nozzle: each one's diameter and volume type (the type's number).
    let diameters: Vec<Value> = variables.list("nozzle_diameter").map_or_else(
        || vec![Value::Float(0.4)],
        |list| {
            list.iter()
                .map(|value| Value::text(value.to_string()))
                .collect()
        },
    );
    variables.set_list("nozzle_diameter_at_nozzle_id", diameters);
    let volume_types = (0..nozzles)
        .map(|nozzle| {
            variables
                .list("nozzle_volume_type")
                .and_then(|list| list.get(nozzle))
                .and_then(|value| value.to_string().trim().parse::<i64>().ok())
                .map_or(int(0), int)
        })
        .collect();
    variables.set_list("nozzle_volume_types", volume_types);
    for name in [
        "current_nozzle_id",
        "next_nozzle_id",
        "initial_nozzle_id",
        "initial_no_support_nozzle_id",
        "curr_physical_extruder_id",
        "most_used_physical_extruder_id",
    ] {
        variables.set(name, int(0));
    }
    variables.set("new_extruder_retracted_length", Value::Float(0.0));
    variables.set("initial_no_support_filament_id", int(first));
    variables.set("initial_filament_id", int(first));
    for name in [
        "initial_no_support_hotend",
        "initial_hotend",
        "current_hotend",
        "next_hotend",
    ] {
        variables.set(name, int(-1));
    }
    variables.set("farthest_point_timelapse_enabled", Value::Bool(false));
    variables.set("timelapse_inline_photo", Value::Bool(false));
    variables.set_list("first_filaments", vec![int(first); nozzles]);
    variables.set_list("first_non_support_filaments", vec![int(first); nozzles]);
    variables.set_list("first_non_support_hotend", vec![int(-1); nozzles]);
    variables.set("wipe_tower_center_pos_valid", Value::Bool(false));
    variables.set("wipe_tower_center_pos_x", Value::Float(0.0));
    variables.set("wipe_tower_center_pos_y", Value::Float(0.0));
    // The change G-code's own are set at each change; these stand until then.
    if !variables.contains("current_filament_id") {
        variables.set("current_filament_id", int(0));
    }
    if !variables.contains("next_filament_id") {
        variables.set("next_filament_id", int(0));
    }
    // No first-layer ramp of the auxiliary fan, the chamber fan at full.
    variables.set("max_additional_fan", int(100));
    variables.set_list(
        "close_additional_fan_first_x_layers",
        vec![int(0); filaments],
    );
    variables.set_list("additional_fan_full_speed_layer", vec![int(1); filaments]);
    variables.set_list(
        "first_x_layer_fan_speed",
        vec![Value::Float(0.0); filaments],
    );
}

/// The names the reference gives a fixed value where Bambu Studio works
/// one out: a template that uses one gets a note ([`fixed_note`]).
pub(crate) const FIXED: [&str; 3] = [
    "has_tpu_in_first_layer",
    "cooling_filter_enabled",
    "hold_chamber_temp_for_flat_print",
];

/// A note naming the fixed values of [`single_nozzle`] the printer's
/// start G-code uses, None where it uses none.
pub(crate) fn fixed_note(settings: &edge_config::Settings) -> Option<String> {
    let start = settings
        .texts("machine_start_gcode")
        .and_then(|texts| texts.first())?;
    let used: Vec<String> = FIXED
        .iter()
        .map(|name| format!("{name} false"))
        .chain(std::iter::once("max_print_z 0".to_owned()))
        .filter(|named| start.contains(named.split(' ').next().unwrap_or_default()))
        .collect();
    (!used.is_empty()).then(|| {
        format!(
            "machine_start_gcode: {} as the reference gives them; Bambu Studio works them out",
            used.join(", ")
        )
    })
}

/// Whether the reference works out a filament grouping for the print
/// (ToolOrdering.cpp `reorder_extruders_for_minimum_flush_volume`, gated
/// on PrintConfig.cpp `support_different_extruders`): a printer of one or
/// two extruders whose `extruder_variant_list` names more than one
/// variant (the H2S, P2S and two-nozzle Bambu Lab printers), unless it
/// prints more than one object one after another. Such a print's file
/// marks its first filament after the start G-code (`;VT<filament> H-1`,
/// GCode.cpp `_do_export`; H is -1 with no nozzle rack).
pub(crate) fn nozzle_grouping(variables: &Variables, objects_in_turn: usize) -> bool {
    let extruders = variables.list("nozzle_diameter").map_or(0, <[Value]>::len);
    let by_object = variables
        .value("print_sequence")
        .is_some_and(|sequence| sequence.to_string() == "by object");
    if extruders == 0 || extruders > 2 || (by_object && objects_in_turn > 1) {
        return false;
    }
    let mut variants: Vec<String> = variables
        .list("extruder_variant_list")
        .unwrap_or(&[])
        .iter()
        .take(extruders)
        .flat_map(|list| {
            list.to_string()
                .split(',')
                .filter(|variant| !variant.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    variants.sort();
    variants.dedup();
    variants.len() > 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(values: &[&str]) -> Vec<Value> {
        values.iter().map(|text| Value::text(*text)).collect()
    }

    /// The flush placeholders as the reference works them out: the
    /// filament's own value, its maximum volumetric speed or the top of its
    /// temperature range where that is 0 (or `nil`), the fast temperature
    /// with `prime_volume_mode` Fast.
    #[test]
    fn flush_placeholders_fall_back_as_the_reference_does() {
        let mut variables = Variables::default();
        variables.set_list(
            "filament_flush_volumetric_speed",
            texts(&["0", "18", "nil"]),
        );
        variables.set_list(
            "filament_max_volumetric_speed",
            texts(&["21", "40", "12.5"]),
        );
        variables.set_list("filament_flush_temp", texts(&["0", "250", "nil"]));
        variables.set_list("filament_flush_temp_fast", texts(&["260", "0", "0"]));
        variables.set_list(
            "nozzle_temperature_range_high",
            texts(&["240", "270", "230"]),
        );
        flush(&mut variables, 3);
        fn read(variables: &Variables, name: &str) -> Vec<String> {
            variables
                .list(name)
                .unwrap()
                .iter()
                .map(Value::to_string)
                .collect()
        }
        assert_eq!(
            read(&variables, "flush_volumetric_speeds"),
            ["21", "18", "12.5"]
        );
        assert_eq!(
            read(&variables, "flush_temperatures"),
            ["240", "250", "230"]
        );
        // A template divides the speed as a decimal.
        assert_eq!(
            edge_template::expand("{flush_volumetric_speeds[0]/2}", &variables).unwrap(),
            "10.5"
        );
        variables.set("prime_volume_mode", Value::text("Fast"));
        flush(&mut variables, 3);
        assert_eq!(
            read(&variables, "flush_temperatures"),
            ["260", "270", "230"]
        );
    }

    /// One nozzle: every filament on it, the first filament named as the
    /// first, the spots that need Bambu Studio's picker left out.
    #[test]
    fn one_nozzle_names() {
        let mut variables = Variables::default();
        variables.set_list("nozzle_diameter", texts(&["0.4"]));
        variables.set_list("filament_vendor", texts(&["Generic", "Bambu Lab"]));
        variables.set("print_sequence", Value::text("by layer"));
        single_nozzle(&mut variables, 2, 1, &[1], true);
        let expand =
            |variables: &Variables, text: &str| edge_template::expand(text, variables).unwrap();
        assert_eq!(
            expand(
                &variables,
                "{filament_map[1]} {initial_no_support_filament_id} {nozzle_diameter_at_nozzle_id[initial_nozzle_id]}"
            ),
            "1 1 0.4"
        );
        assert_eq!(
            expand(
                &variables,
                "{is_all_bbl_filament} {max_print_z} {clear_to_x0}"
            ),
            "true 0 true"
        );
        assert!(edge_template::expand("{has_timelapse_safe_pos}", &variables).is_err());
        single_nozzle(&mut variables, 2, 0, &[0, 1], false);
        assert_eq!(expand(&variables, "{is_all_bbl_filament}"), "false");
    }

    /// The H2S and P2S list three extruder variants: the reference groups
    /// their filaments and marks the first; an X1 Carbon lists one.
    #[test]
    fn grouping_printers() {
        let mut variables = Variables::default();
        variables.set_list("nozzle_diameter", texts(&["0.4"]));
        variables.set_list(
            "extruder_variant_list",
            texts(&["Direct Drive Standard,Direct Drive High Flow"]),
        );
        assert!(nozzle_grouping(&variables, 1));
        variables.set("print_sequence", Value::text("by object"));
        assert!(nozzle_grouping(&variables, 1));
        assert!(!nozzle_grouping(&variables, 2));
        variables.set_list("extruder_variant_list", texts(&["Direct Drive Standard"]));
        assert!(!nozzle_grouping(&variables, 1));
    }
}
/// Whether `key` is one of the bed-slinger mass model's two settings and
/// the printer models it: the reference lists them in the file's settings
/// only then (GCode.cpp, `dump_mass_model_keys`; the A2L sets both).
pub(crate) fn listed_mass_model(settings: &edge_config::Settings, key: &str) -> bool {
    const KEYS: [&str; 2] = ["machine_max_force_Y", "machine_bed_mass_Y"];
    KEYS.contains(&key)
        && KEYS.iter().any(|key| {
            settings
                .float(key)
                .ok()
                .flatten()
                .is_some_and(|value| value > 0.0)
        })
}

/// The Y acceleration a bed-slinger's motor can give the bed and the part
/// on it (GCode.cpp `mass_load_limited_machine_acceleration`, Bambu
/// Studio's): `machine_max_force_Y` (N) over `machine_bed_mass_Y` plus the
/// mass printed so far (g), at most the lowest `machine_max_acceleration_y`;
/// that lowest alone where either is unset (0) or nothing is printed yet.
/// In mm/s².
pub(crate) fn y_acceleration_limit(force: f64, bed_mass: f64, printed: f64, limit_y: f64) -> f64 {
    const EPSILON: f64 = 1e-4;
    if force > EPSILON && bed_mass > EPSILON && printed > EPSILON {
        // 1 N = 1e6 g mm/s².
        (force * 1e6 / (bed_mass + printed)).min(limit_y)
    } else {
        limit_y
    }
}

#[cfg(test)]
mod mass_tests {
    use super::y_acceleration_limit;

    #[test]
    fn the_y_limit_falls_as_the_part_grows() {
        // The A2L: 29 N over its 2700 g bed.
        assert_eq!(y_acceleration_limit(29.0, 2700.0, 0.0, 8000.0), 8000.0);
        assert_eq!(y_acceleration_limit(29.0, 2700.0, 900.0, 8000.0), 8000.0);
        let heavy = y_acceleration_limit(29.0, 2700.0, 1300.0, 8000.0);
        assert!((heavy - 29e6 / 4000.0).abs() < 1e-9);
        // Either setting unset: the machine's limit.
        assert_eq!(y_acceleration_limit(0.0, 2700.0, 1300.0, 8000.0), 8000.0);
        assert_eq!(y_acceleration_limit(29.0, 0.0, 1300.0, 8000.0), 8000.0);
    }
}

/// Where a layer reaches farthest from the camera, at the bed's origin
/// (GCode.cpp `compute_farthest_point`, Bambu Studio 2.8's): the point of
/// an outer wall farthest from it, or, with no outer wall on the layer,
/// of the sparse, solid and top fill and the support. None on a layer
/// with none of these. The first of equally far points.
pub(crate) fn farthest_point(parts: &[crate::Layer<'_>]) -> Option<edge_geom::Point2> {
    use edge_extrusion::Role;
    fn farther(best: &mut Option<(i128, edge_geom::Point2)>, point: edge_geom::Point2) {
        let squared = i128::from(point.x).pow(2) + i128::from(point.y).pow(2);
        if squared > 0 && best.is_none_or(|(most, _)| squared > most) {
            *best = Some((squared, point));
        }
    }
    let mut walls = None;
    let mut rest = None;
    for part in parts {
        for wall in part
            .loops
            .iter()
            .filter(|wall| wall.role == Role::ExternalPerimeter)
        {
            for point in &wall.polygon.points {
                farther(&mut walls, *point);
            }
        }
        let fill = part.paths.iter().filter(|path| {
            matches!(
                path.role,
                Role::SparseInfill | Role::SolidInfill | Role::TopSolidInfill
            )
        });
        let support = part
            .support
            .iter()
            .filter(|path| matches!(path.role, Role::Support | Role::SupportInterface));
        for path in fill.chain(support) {
            for point in &path.points {
                farther(&mut rest, *point);
            }
        }
    }
    walls.or(rest).map(|(_, point)| point)
}
