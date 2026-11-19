//! What the rewritten Type 1 tower reads: WipeTower.cpp's constructor and
//! `set_extruder`, and what Print.cpp `_make_wipe_tower` hands it
//! (`set_first_layer_flow_ratio`, `bambu_prime_volume`).
//!
//! Figures the tower works in single precision are kept as `f32`, as
//! there, so that what is planned from them lands where the reference's
//! does.

use edge_config::{ConfigError, FloatOrPercent, Settings as Config};

/// The tower's lines are this many nozzle diameters wide
/// (WipeTower.hpp `Width_To_Nozzle_Ratio`).
pub(crate) const WIDTH_TO_NOZZLE: f32 = 1.25;

/// What stands round the tower (`wipe_tower_wall_type`). The cone is a
/// WipeTower2 wall: this tower lays a box for it, as the reference's does
/// (Print.cpp, at `construct_mesh`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wall {
    Box,
    Ribs {
        /// `wipe_tower_rib_width`.
        width: f32,
        /// `wipe_tower_extra_rib_length`.
        extra: f32,
        /// `wipe_tower_fillet_wall`.
        fillet: bool,
    },
}

/// `prime_volume_mode` (PrintConfig.cpp `PrimeVolumeMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimeVolumeMode {
    Default,
    /// Every change primes 15 mm³ (WipeTower.cpp `bambu_prime_volume`).
    Saving,
    /// Flushes by `flush_multiplier_fast` (ToolOrdering.cpp); the tower
    /// primes as in `Default`.
    Fast,
}

/// One filament as the tower knows it (WipeTower.hpp `FilamentParameters`).
#[derive(Clone, Debug, PartialEq)]
pub struct Filament {
    /// `filament_type`.
    pub material: String,
    /// `filament_is_support`.
    pub is_support: bool,
    /// `filament_soluble`, or with `wipe_tower_filament` set every
    /// filament but that one.
    pub is_soluble: bool,
    /// The block it purges into: `filament_adhesiveness_category`, or
    /// failing that the one Bambu's own presets give its type
    /// ([`adhesiveness_category`]).
    pub category: i32,
    /// `filament_prime_volume` and `filament_prime_volume_nc`, in mm³.
    pub prime_volume: f32,
    pub prime_volume_nc: f32,
    /// `filament_diameter`, and its cross-section in mm².
    pub diameter: f32,
    pub area: f32,
    /// `filament_max_volumetric_speed` in mm³/s; 0 for none.
    pub max_volumetric_speed: f32,
    /// `nozzle_temperature` and `nozzle_temperature_initial_layer`.
    pub temperature: i32,
    pub first_layer_temperature: i32,
    /// `wipe_distance`: how far the nozzle wipes back along a ribbed wall.
    pub wipe_distance: f32,
    /// `retraction_length` and `retraction_speed`, round the ironing at a
    /// wall gap.
    pub retract_length: f32,
    pub retract_speed: f32,
    /// `filament_tower_ironing_area`, in mm².
    pub iron_area: f32,
    /// `filament_prime_tower_speed_factor_curve`, each step clamped to
    /// 0.01..5; empty for the tower's own ramp.
    pub speed_factor_curve: Vec<f32>,
    /// `filament_cooling_before_tower`, in °C.
    pub cooling_before_tower: f32,
    /// The extruder it prints from, from 0 (`filament_map`, which the
    /// grouping sets on a printer of two nozzles; [`Settings::map_extruders`]).
    pub extruder: usize,
    /// What it rams on the tower when it leaves its nozzle, in mm of
    /// filament: for a change onto the other extruder, and for one onto
    /// another hotend of its own (`filament_change_length`, `_nc`).
    pub change_length: (f32, f32),
    /// How fast it is rammed, in mm/s of filament
    /// (`filament_ramming_volumetric_speed`, `_nc`; -1 or nil is its
    /// `filament_max_volumetric_speed`).
    pub ramming_speed: (f32, f32),
    /// How long the nozzle travels back over the ramming lines after it
    /// (`filament_ramming_travel_time`, `_nc`), in seconds.
    pub ramming_travel_time: (f32, f32),
    /// The temperature the nozzle put away is cooled to while it rams
    /// (`filament_pre_cooling_temperature`, `_nc`, with
    /// `enable_pre_heating`; 0 for none), and how long that takes on each
    /// extruder (by `hotend_cooling_rate`), on the first layer and above.
    pub precool_target: (i32, i32),
    pub precool_seconds: (Vec<f32>, Vec<f32>),
    pub precool_seconds_first: (Vec<f32>, Vec<f32>),
    /// The tower interface's temperature (`filament_tower_interface_print_temp`,
    /// -1 for `nozzle_temperature_range_high`), how far outside the tower
    /// a contact purge starts (`filament_tower_interface_pre_extrusion_dist`)
    /// and what it primes more there (`_length`).
    pub interface_temperature: i32,
    pub run_in_distance: f32,
    pub extra_prime_length: f32,
}

/// Everything the Type 1 tower reads. Defaults are the reference's
/// (PrintConfig.cpp). Lengths are in mm, speeds in mm/min where the tower
/// works in them.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// `wipe_tower_x` / `wipe_tower_y` of the plate: where the tower's
    /// frame starts (with a ribbed wall, the corner of the box round its
    /// first layer's wall).
    pub x: f32,
    pub y: f32,
    /// `prime_tower_width`.
    pub width: f32,
    /// `prime_tower_brim_width`; below 0 is "auto", by the tower's height.
    pub brim_width: f32,
    /// The first nozzle's diameter (`nozzle_diameter`).
    pub nozzle_diameter: f32,
    /// The tower's line width: `prime_tower_line_width`, or 1.25 nozzle
    /// diameters where that is 0 (`m_perimeter_width`).
    pub line_width: f32,
    /// `prime_tower_infill_gap`, as a share of a line width (1.5 = 150%).
    pub infill_gap: f32,
    /// `prime_tower_enable_framework`: every layer as deep as the first.
    pub framework: bool,
    /// `prime_tower_line_ironing`, `prime_tower_flat_ironing` (only with
    /// the wall gap), and `prime_tower_ironing_speed` in mm/min.
    pub line_ironing: bool,
    pub flat_ironing: bool,
    pub ironing_speed: f32,
    /// `wipe_tower_wall_gap` (Bambu's `prime_tower_skip_points`).
    pub wall_gap: bool,
    pub wall: Wall,
    /// `wipe_tower_max_purge_speed` (Bambu's `prime_tower_max_speed`), in
    /// mm/min.
    pub max_speed: f32,
    /// `initial_layer_speed` in mm/s, 30 where it is 0.
    pub first_layer_speed: f32,
    /// `travel_speed` in mm/s.
    pub travel_speed: f32,
    /// The first layer's purge flow: the process' `bottom_solid_infill_flow_ratio`
    /// (Bambu's `initial_layer_flow_ratio`; Print.cpp `set_first_layer_flow_ratio`).
    pub first_layer_flow_ratio: f32,
    /// `prime_volume`, which a filament without `filament_prime_volume`
    /// primes.
    pub prime_volume: f32,
    pub prime_volume_mode: PrimeVolumeMode,
    /// `flush_multiplier`.
    pub flush_multiplier: f32,
    /// The layers with no tool change are left out
    /// (`wipe_tower_sparse_layers_skipped`), or laid as one
    /// (`wipe_tower_sparse_layers_combined`).
    pub skip_sparse: bool,
    pub combine_sparse: bool,
    /// `timelapse_type` smooth: the tower stands on every layer.
    pub smooth_timelapse: bool,
    /// `enable_arc_fitting`, not for a rotated tower.
    pub arc_fitting: bool,
    /// `wipe_tower_rotation_angle`, in degrees.
    pub rotation: f32,
    /// `prime_tower_feedrate_factor`, at least 0.01.
    pub feedrate_factor: f32,
    /// `prime_tower_acceleration`, rounded; 0 for none.
    pub acceleration: u32,
    /// `extruder_printable_height` per extruder; 0 is no limit.
    pub printable_height: Vec<f64>,
    /// `enable_tower_interface_features`, where the reference acts on it
    /// ([`interface_features_supported`]).
    pub interface_features: bool,
    /// `enable_tower_interface_cooldown_during_tower`: the print
    /// temperature goes back on before a contact block's rows, not after.
    pub interface_cooldown: bool,
    /// A filament switcher on extruders of two kinds
    /// (`has_filament_switcher`, `extruder_type`): the change cools 10 °C
    /// more before the tower, which is not done.
    pub mixed_extruders: bool,
    /// `prime_tower_side_scrape`: the nozzle scrapes on the tower's side
    /// before a purge (Bambu Studio 2.9.1's, set by the H3C's presets).
    pub side_scrape: bool,
    /// Any of Edge's own tower interface options is on
    /// (`wipe_tower_interface_temp`, `wipe_tower_interface_run_in` with the
    /// wall gap, `wipe_tower_interface_extra_prime`;
    /// WipeTowerInterface.cpp `Settings::from_config`). Bambu's switch
    /// turns them off where it acts.
    pub edge_interface: bool,
    /// A Bambu Lab printer (`printer_model`, or failing that the preset's
    /// name; Print.cpp `is_BBL_printer`). Another printer set to this tower
    /// (`wipe_tower_type` `type1`) primes `prime_volume` on every change.
    pub bambu: bool,
    /// How many nozzles the printer has (`nozzle_diameter`), and whether
    /// each rack hotend counts as an extruder (`handle_hotend_as_extruder`).
    pub nozzles: usize,
    pub hotend_as_extruder: bool,
    /// Each extruder's nozzle diameter (`nozzle_diameter`), and its
    /// physical extruder, the one a temperature line names
    /// (`physical_extruder_map`).
    pub nozzle_diameters: Vec<f32>,
    pub physical_extruders: Vec<usize>,
    /// The shared part of the bed both nozzles reach, min x, min y, max x,
    /// max y (`printable_area` cut by `extruder_printable_area`;
    /// WipeTowerInterface.cpp `shared_printable_box`); none for no bed.
    pub shared_bed: Option<[f64; 4]>,
    pub filaments: Vec<Filament>,
    /// What is read but not followed as the reference follows it, each
    /// with what is done instead.
    pub notes: Vec<String>,
}

/// Whether `enable_tower_interface_features` does anything
/// (WipeTowerInterface.cpp `bambu_features_supported`): only on printers of
/// more than one nozzle, or whose rack hotends count as extruders. Nothing
/// reads it on a single-nozzle Bambu printer or the H2S.
pub fn interface_features_supported(config: &Config) -> Result<bool, ConfigError> {
    Ok(config.flag("handle_hotend_as_extruder")?.unwrap_or(false)
        || config
            .texts("nozzle_diameter")
            .is_some_and(|nozzles| nozzles.len() > 1))
}

/// The part of the bed every nozzle reaches, as a box (WipeTowerInterface.cpp
/// `shared_printable_box`): the box round `printable_area`, cut to the box
/// round each `extruder_printable_area` of three corners or more.
fn shared_printable_box(config: &Config) -> Option<[f64; 4]> {
    let point = |text: &str| -> Option<(f64, f64)> {
        let (x, y) = text.trim().split_once('x')?;
        Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
    };
    let bounds = |points: &[(f64, f64)]| -> Option<[f64; 4]> {
        let first = points.first()?;
        Some(points.iter().fold(
            [first.0, first.1, first.0, first.1],
            |[a, b, c, d], (x, y)| [a.min(*x), b.min(*y), c.max(*x), d.max(*y)],
        ))
    };
    let bed: Vec<(f64, f64)> = config
        .texts("printable_area")
        .unwrap_or_default()
        .iter()
        .filter_map(|text| point(text))
        .collect();
    let mut shared = bounds(&bed)?;
    for group in config.texts("extruder_printable_area").unwrap_or_default() {
        let points: Vec<(f64, f64)> = group.split(',').filter_map(point).collect();
        if points.len() < 3 {
            continue;
        }
        if let Some([a, b, c, d]) = bounds(&points) {
            shared = [
                shared[0].max(a),
                shared[1].max(b),
                shared[2].min(c),
                shared[3].min(d),
            ];
        }
    }
    Some(shared)
}

/// The block a filament purges into (WipeTower.cpp `adhesiveness_category`):
/// its own `filament_adhesiveness_category`, or for 0 the one Bambu
/// Studio's presets give its type, so that an uncategorised PLA and PETG
/// do not share a block. A type without one stays 0.
pub fn adhesiveness_category(configured: i32, material: &str, is_support: bool) -> i32 {
    if configured != 0 {
        return configured;
    }
    let t = material.trim().to_ascii_uppercase();
    let starts = |prefix: &str| t.starts_with(prefix);
    match t.as_str() {
        "PVA" => return 704,
        "BVOH" => return 797,
        "HIPS" => return 798,
        _ => {}
    }
    if is_support {
        return if starts("PLA") {
            702
        } else if starts("PA") || starts("PET") {
            703
        } else if starts("ABS") || starts("ASA") {
            706
        } else {
            0
        };
    }
    if starts("PLA") {
        100
    } else if starts("ABS") || starts("ASA") {
        200
    } else if t == "PET-CF" {
        800
    } else if starts("PETG") || starts("PCTG") || t == "PET" {
        300
    } else if starts("PPS") {
        801
    } else if starts("PPA") {
        400
    } else if t == "PP" || starts("PP-") {
        902
    } else if starts("PA") {
        400
    } else if t == "PC" || starts("PC-") || starts("PC/") {
        500
    } else if starts("TPU") || starts("TPE") || t == "FLEX" {
        600
    } else if t == "PE" || starts("PE-") {
        901
    } else {
        0
    }
}

/// `filament_prime_tower_speed_factor_curve` (WipeTower.cpp
/// `parse_speed_factor_curve`): numbers split at commas, each read from its
/// start as far as it is a number and clamped to 0.01..5. None where a
/// piece does not start with one.
fn speed_factor_curve(text: &str) -> Option<Vec<f32>> {
    let text = text.trim_matches('"');
    if text.is_empty() {
        return Some(Vec::new());
    }
    text.split(',')
        .map(|piece| {
            let piece = piece.trim_start_matches([' ', '\t']);
            // The longest start of the piece that reads as a number
            // (`string_to_double_decimal_point` stops where it stops).
            (1..=piece.len())
                .rev()
                .filter(|end| piece.is_char_boundary(*end))
                .find_map(|end| piece[..end].parse::<f64>().ok())
                .map(|value| (value as f32).clamp(0.01, 5.0))
        })
        .collect()
}

impl Settings {
    /// Reads what is present; anything absent keeps the reference's
    /// default. The tower's place is kept per plate, and `plate` picks
    /// which.
    pub fn from_settings(config: &Config, plate: usize) -> Result<Self, ConfigError> {
        let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(config.float(key)?.unwrap_or(default))
        };
        let list = |key: &str| -> Result<Vec<Option<f64>>, ConfigError> {
            Ok(config.floats(key)?.unwrap_or_default())
        };
        // `ConfigOptionVector::get_at`: past the end, the first.
        let at = |values: &[Option<f64>], index: usize, default: f64| -> f64 {
            values
                .get(index)
                .or(values.first())
                .copied()
                .flatten()
                .unwrap_or(default)
        };
        let flag = |key: &str, default: bool| -> Result<bool, ConfigError> {
            Ok(config.flag(key)?.unwrap_or(default))
        };
        let placed = |key: &str, default: f64| -> Result<f64, ConfigError> {
            let values = list(key)?;
            Ok(values
                .get(plate)
                .or(values.last())
                .copied()
                .flatten()
                .unwrap_or(default))
        };
        // A speed the process may give per extruder: the first
        // (`get_value_at` for the first filament on one nozzle).
        let first_of = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(list(key)?.first().copied().flatten().unwrap_or(default))
        };
        // Bambu Studio names some of the tower's settings its own way; its
        // project is read under those names, which the reference takes over
        // ours as it loads it (Format/BambuKeyAliases.cpp).
        let alias = |ours: &'static str, theirs: &'static str| -> &'static str {
            if config.text(theirs).is_some() {
                theirs
            } else {
                ours
            }
        };
        let mut notes = Vec::new();

        let nozzles: Vec<Option<f64>> = list("nozzle_diameter")?;
        let nozzle_diameter = nozzles.first().copied().flatten().unwrap_or(0.4) as f32;
        let tower_line_width = number("prime_tower_line_width", 0.0)?.max(0.0) as f32;
        let line_width = if tower_line_width > 0.0 {
            tower_line_width
        } else {
            nozzle_diameter * WIDTH_TO_NOZZLE
        };
        let infill_gap = match config.float_or_percent("prime_tower_infill_gap")? {
            Some(FloatOrPercent::Percent(share) | FloatOrPercent::Value(share)) => share / 100.0,
            None => 1.5,
        } as f32;
        let wall_gap = flag("wipe_tower_wall_gap", true)?;
        let wall = match config.text(alias("wipe_tower_wall_type", "prime_tower_rib_wall")) {
            Some("rib" | "1") => Wall::Ribs {
                width: number(alias("wipe_tower_rib_width", "prime_tower_rib_width"), 8.0)? as f32,
                extra: number(
                    alias(
                        "wipe_tower_extra_rib_length",
                        "prime_tower_extra_rib_length",
                    ),
                    0.0,
                )? as f32,
                fillet: flag(
                    alias("wipe_tower_fillet_wall", "prime_tower_fillet_wall"),
                    true,
                )?,
            },
            _ => Wall::Box,
        };
        let timelapse_smooth = config.text("timelapse_type") == Some("1");
        let no_sparse = flag("wipe_tower_no_sparse_layers", false)?;
        let skip_sparse = no_sparse && !timelapse_smooth;
        let combine_sparse = flag("wipe_tower_sparse_layers_combination", false)?
            && !skip_sparse
            && !timelapse_smooth;
        let rotation = number("wipe_tower_rotation_angle", 0.0)? as f32;
        let prime_volume_mode = match config.text("prime_volume_mode") {
            None | Some("Default") => PrimeVolumeMode::Default,
            Some("Saving") => PrimeVolumeMode::Saving,
            Some("Fast") => PrimeVolumeMode::Fast,
            Some(other) => {
                return Err(ConfigError::BadValue {
                    key: "prime_volume_mode".to_owned(),
                    found: other.to_owned(),
                    expected: "Default, Saving or Fast",
                });
            }
        };
        let first_layer_speed = Some(first_of("initial_layer_speed", 30.0)?)
            .filter(|speed| *speed != 0.0)
            .unwrap_or(30.0) as f32;

        // The filaments, as many as `filament_diameter` lists (PrintConfig.hpp
        // `flow_variant_filament_count`), else as `filament_type`.
        let types: Vec<String> = config
            .texts("filament_type")
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let diameters = list("filament_diameter")?;
        let count = if diameters.is_empty() {
            types.len().max(1)
        } else {
            diameters.len()
        };
        let flags = |key: &str| -> Vec<bool> {
            config
                .texts(key)
                .unwrap_or_default()
                .iter()
                .map(|flag| flag == "1" || flag == "true")
                .collect()
        };
        let flag_at = |values: &[bool], index: usize| -> bool {
            values
                .get(index)
                .or(values.first())
                .copied()
                .unwrap_or(false)
        };
        let soluble = flags("filament_soluble");
        let support = flags("filament_is_support");
        let forced = number("wipe_tower_filament", 0.0)? as usize;
        let categories = list("filament_adhesiveness_category")?;
        let prime = list("filament_prime_volume")?;
        let prime_nc = list("filament_prime_volume_nc")?;
        let volumetric = list("filament_max_volumetric_speed")?;
        let temperatures = list("nozzle_temperature")?;
        let first_temperatures = list("nozzle_temperature_initial_layer")?;
        // A filament's own retraction settings over the printer's, entry by
        // entry, `nil` taking the printer's (PrintApply.cpp
        // `print_config_diffs`, `filament_<key>`), as the tower reads them.
        let overridden = |key: &str| -> Result<Vec<Option<f64>>, ConfigError> {
            let printer = list(key)?;
            let own = list(&format!("filament_{key}"))?;
            if own.iter().all(Option::is_none) {
                return Ok(printer);
            }
            let first = printer.first().copied().flatten();
            Ok(own.into_iter().map(|value| value.or(first)).collect())
        };
        let wipe = overridden("wipe_distance")?;
        let retract = overridden("retraction_length")?;
        let retract_speed = overridden("retraction_speed")?;
        let iron = list("filament_tower_ironing_area")?;
        let cooling = list("filament_cooling_before_tower")?;
        let curves: Vec<String> = config
            .texts("filament_prime_tower_speed_factor_curve")
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let prime_volume = number("prime_volume", 45.0)? as f32;
        // The nozzle-change ramming (WipeTower.cpp `set_extruder`, Bambu
        // Studio 2.9.1's): `.0` for a change onto the other extruder, `.1`
        // for one onto another hotend of the same extruder.
        let change_length = (
            list("filament_change_length")?,
            list("filament_change_length_nc")?,
        );
        let ramming_speed = (
            list("filament_ramming_volumetric_speed")?,
            list("filament_ramming_volumetric_speed_nc")?,
        );
        let ramming_travel = (
            list("filament_ramming_travel_time")?,
            list("filament_ramming_travel_time_nc")?,
        );
        let precool = (
            list("filament_pre_cooling_temperature")?,
            list("filament_pre_cooling_temperature_nc")?,
        );
        let pre_heating = flag("enable_pre_heating", false)?;
        let cooling_rates: Vec<f32> = list("hotend_cooling_rate")?
            .into_iter()
            .map(|rate| rate.filter(|r| r.is_finite()).unwrap_or(0.0) as f32)
            .collect();
        let range_high = list("nozzle_temperature_range_high")?;
        let interface_temps = list("filament_tower_interface_print_temp")?;
        let run_in = list("filament_tower_interface_pre_extrusion_dist")?;
        let extra_prime = list("filament_tower_interface_pre_extrusion_length")?;
        let nozzle_diameters: Vec<f32> = nozzles.iter().map(|d| d.unwrap_or(0.4) as f32).collect();
        let extruders = nozzle_diameters.len().max(1);
        let mut filaments = Vec::with_capacity(count);
        for index in 0..count {
            let material = types
                .get(index)
                .or(types.first())
                .cloned()
                .unwrap_or_else(|| "PLA".to_owned());
            let is_support = flag_at(&support, index);
            let diameter = at(&diameters, index, 1.75) as f32;
            let max_volumetric_speed = at(&volumetric, index, 2.0) as f32;
            let area = std::f32::consts::PI / 4.0 * diameter * diameter;
            // Nil or -1 rams at the filament's fastest.
            let ram_speed = |values: &[Option<f64>]| {
                let own = if values.is_empty() {
                    -1.0
                } else {
                    values
                        .get(index)
                        .or(values.first())
                        .copied()
                        .flatten()
                        .filter(|v| !v.is_nan())
                        .unwrap_or(-1.0) as f32
                };
                let volume = if (own + 1.0).abs() < 1e-6 || own <= 0.0 {
                    max_volumetric_speed
                } else {
                    own
                };
                if volume > 0.0 && area > 0.0 {
                    volume / area
                } else {
                    0.0
                }
            };
            let temperature = at(&temperatures, index, 0.0) as i32;
            let first_layer_temperature = at(&first_temperatures, index, 0.0) as i32;
            let targets = (
                at(&precool.0, index, 0.0) as i32,
                at(&precool.1, index, 0.0) as i32,
            );
            // `set_extruder`'s pre-cooling times, by extruder.
            let seconds = |target: i32, from: i32| -> Vec<f32> {
                (0..extruders)
                    .map(|e| {
                        let rate = cooling_rates.get(e).copied().unwrap_or(0.0);
                        if !pre_heating || target == 0 || rate <= 0.0 {
                            0.0
                        } else {
                            (from - target).max(0) as f32 / rate
                        }
                    })
                    .collect()
            };
            let interface_temperature = {
                let configured = at(&interface_temps, index, -1.0) as i32;
                let t = if configured == -1 {
                    at(&range_high, index, 0.0) as i32
                } else {
                    configured
                };
                if t > 0 { t } else { temperature }
            };
            let curve_text = curves
                .get(index)
                .or(curves.first())
                .map_or("", String::as_str);
            let speed_factor_curve = match speed_factor_curve(curve_text) {
                Some(curve) => curve,
                None => {
                    notes.push(format!(
                        "filament_prime_tower_speed_factor_curve \"{curve_text}\": not a list of \
                         numbers; filament {} purges on the tower's own speed ramp",
                        index + 1
                    ));
                    Vec::new()
                }
            };
            filaments.push(Filament {
                category: adhesiveness_category(
                    at(&categories, index, 0.0) as i32,
                    &material,
                    is_support,
                ),
                material,
                is_support,
                is_soluble: if forced > 0 {
                    index != forced - 1
                } else {
                    flag_at(&soluble, index)
                },
                prime_volume: if prime.is_empty() {
                    prime_volume
                } else {
                    at(&prime, index, 45.0) as f32
                },
                prime_volume_nc: at(&prime_nc, index, 60.0) as f32,
                diameter,
                area,
                max_volumetric_speed,
                temperature,
                first_layer_temperature,
                wipe_distance: at(&wipe, index, 1.0) as f32,
                retract_length: at(&retract, index, 0.0) as f32,
                retract_speed: at(&retract_speed, index, 0.0) as f32,
                iron_area: at(&iron, index, 4.0) as f32,
                speed_factor_curve,
                // Nil or absent is none. Bambu's filament switcher adds 10 °C on
                // printers of two kinds of extruder only
                // (PrintConfig.cpp `filament_cooling_before_tower_at`): not the
                // H2D, whose two are alike; such a printer is refused
                // ([`Settings::mixed_extruders`]).
                cooling_before_tower: at(&cooling, index, 0.0) as f32,
                extruder: 0,
                change_length: (
                    (at(&change_length.0, index, 0.0) as f32).max(0.0),
                    (at(&change_length.1, index, 0.0) as f32).max(0.0),
                ),
                ramming_speed: (ram_speed(&ramming_speed.0), ram_speed(&ramming_speed.1)),
                ramming_travel_time: (
                    (at(&ramming_travel.0, index, 0.0) as f32).max(0.0),
                    (at(&ramming_travel.1, index, 0.0) as f32).max(0.0),
                ),
                precool_target: (
                    if pre_heating { targets.0 } else { 0 },
                    if pre_heating { targets.1 } else { 0 },
                ),
                precool_seconds: (
                    seconds(targets.0, temperature),
                    seconds(targets.1, temperature),
                ),
                precool_seconds_first: (
                    seconds(targets.0, first_layer_temperature),
                    seconds(targets.1, first_layer_temperature),
                ),
                interface_temperature,
                run_in_distance: (at(&run_in, index, 10.0) as f32).max(0.0),
                extra_prime_length: (at(&extra_prime, index, 0.0) as f32).max(0.0),
            });
        }
        let physical_extruders: Vec<usize> = list("physical_extruder_map")?
            .into_iter()
            .enumerate()
            .map(|(e, p)| p.map_or(e, |p| p.max(0.0) as usize))
            .collect();
        let shared_bed = shared_printable_box(config);
        let map: Vec<usize> = list("filament_map")?
            .into_iter()
            .map(|e| e.map_or(0, |e| (e as i64 - 1).max(0) as usize))
            .collect();

        let interface_features = flag("enable_tower_interface_features", false)?
            && interface_features_supported(config)?;
        let edge_interface = !interface_features
            && (flag("wipe_tower_interface_temp", false)?
                || (flag("wipe_tower_interface_run_in", false)? && wall_gap)
                || flag("wipe_tower_interface_extra_prime", false)?);
        let mut settings = Self {
            x: placed("wipe_tower_x", 15.0)? as f32,
            y: placed("wipe_tower_y", 220.0)? as f32,
            width: number("prime_tower_width", 60.0)? as f32,
            brim_width: number("prime_tower_brim_width", 3.0)? as f32,
            nozzle_diameter,
            line_width,
            infill_gap,
            framework: flag("prime_tower_enable_framework", false)?,
            line_ironing: flag("prime_tower_line_ironing", true)?,
            flat_ironing: flag("prime_tower_flat_ironing", false)? && wall_gap,
            ironing_speed: number("prime_tower_ironing_speed", 10.0)? as f32 * 60.0,
            wall_gap,
            wall,
            max_speed: number(
                alias("wipe_tower_max_purge_speed", "prime_tower_max_speed"),
                90.0,
            )? as f32
                * 60.0,
            first_layer_speed,
            travel_speed: first_of("travel_speed", 120.0)? as f32,
            first_layer_flow_ratio: number(
                alias("bottom_solid_infill_flow_ratio", "initial_layer_flow_ratio"),
                1.0,
            )? as f32,
            prime_volume,
            prime_volume_mode,
            flush_multiplier: first_of("flush_multiplier", 0.3)? as f32,
            skip_sparse,
            combine_sparse,
            smooth_timelapse: timelapse_smooth,
            arc_fitting: flag("enable_arc_fitting", false)? && rotation.abs() < 1e-4,
            rotation,
            feedrate_factor: (number("prime_tower_feedrate_factor", 1.0)? as f32).max(0.01),
            acceleration: {
                let acceleration = number("prime_tower_acceleration", 0.0)?;
                if acceleration > 0.0 {
                    (acceleration + 0.5).floor() as u32
                } else {
                    0
                }
            },
            printable_height: list("extruder_printable_height")?
                .into_iter()
                .map(|height| height.unwrap_or(0.0))
                .collect(),
            interface_features,
            interface_cooldown: flag("enable_tower_interface_cooldown_during_tower", false)?,
            side_scrape: flag("prime_tower_side_scrape", false)?,
            mixed_extruders: flag("has_filament_switcher", false)?
                && config
                    .texts("extruder_type")
                    .is_some_and(|types| types.windows(2).any(|pair| pair[0] != pair[1])),
            edge_interface,
            bambu: config
                .text("printer_model")
                .filter(|model| !model.is_empty())
                .or(config.text("printer_settings_id"))
                .is_some_and(|name| name.starts_with("Bambu Lab")),
            nozzles: nozzles.len().max(1),
            hotend_as_extruder: flag("handle_hotend_as_extruder", false)?,
            nozzle_diameters,
            physical_extruders,
            shared_bed,
            filaments,
            notes,
        };
        settings.map_extruders(&map);
        Ok(settings)
    }

    /// Each filament's extruder, from 0 (the grouping's `filament_map`);
    /// the first for a filament the list does not name.
    pub fn map_extruders(&mut self, extruders: &[usize]) {
        let count = self.nozzle_diameters.len().max(1);
        for (index, filament) in self.filaments.iter_mut().enumerate() {
            filament.extruder = extruders
                .get(index)
                .copied()
                .filter(|e| *e < count)
                .unwrap_or(0);
        }
    }

    /// The diameter of the nozzle `filament` prints from
    /// (`nozzle_diameter_for_filament`).
    pub fn nozzle_of(&self, filament: usize) -> f32 {
        let extruder = self.filaments.get(filament).map_or(0, |f| f.extruder);
        self.nozzle_diameters
            .get(extruder)
            .or(self.nozzle_diameters.first())
            .copied()
            .unwrap_or(self.nozzle_diameter)
    }

    /// The physical extruder a temperature line for `filament` names
    /// (`reheat_extruder`): its own on a printer of more than one nozzle,
    /// else the first `physical_extruder_map` entry.
    pub fn physical_extruder(&self, filament: usize) -> usize {
        if self.nozzle_diameters.len() > 1 {
            let extruder = self.filaments.get(filament).map_or(0, |f| f.extruder);
            return self
                .physical_extruders
                .get(extruder)
                .copied()
                .unwrap_or(extruder);
        }
        self.physical_extruders.first().copied().unwrap_or(0)
    }

    /// What the tower primes after a change to `filament`
    /// (WipeTower.cpp `bambu_prime_volume`): 15 mm³ when saving, else the
    /// filament's `filament_prime_volume`, or `filament_prime_volume_nc`
    /// after a hotend change on one extruder. On a printer that is not a
    /// Bambu Lab one, `prime_volume` (Print.cpp `_make_wipe_tower`).
    pub fn prime_volume(&self, filament: usize, hotend_change: bool) -> f32 {
        if !self.bambu {
            return self.prime_volume;
        }
        if self.prime_volume_mode == PrimeVolumeMode::Saving {
            return 15.0;
        }
        match self.filaments.get(filament).or(self.filaments.first()) {
            Some(own) if hotend_change => own.prime_volume_nc,
            Some(own) => own.prime_volume,
            None => self.prime_volume,
        }
    }

    /// The filament's block, by its category (WipeTower.hpp
    /// `get_filament_category`).
    pub fn category(&self, filament: usize) -> i32 {
        self.filaments.get(filament).map_or(0, |own| own.category)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(json: &str) -> Settings {
        Settings::from_settings(&Config::from_json(json).unwrap(), 0).unwrap()
    }

    #[test]
    fn absent_settings_are_the_reference_defaults() {
        let s = read("{}");
        assert_eq!((s.x, s.y, s.width, s.brim_width), (15.0, 220.0, 60.0, 3.0));
        assert_eq!(s.line_width, 0.5);
        assert_eq!(s.infill_gap, 1.5);
        assert!(!s.framework && s.line_ironing && !s.flat_ironing && s.wall_gap);
        assert_eq!(s.ironing_speed, 600.0);
        assert_eq!(s.wall, Wall::Box);
        assert_eq!(s.max_speed, 5400.0);
        assert_eq!(s.first_layer_speed, 30.0);
        assert_eq!(s.first_layer_flow_ratio, 1.0);
        assert_eq!(s.prime_volume_mode, PrimeVolumeMode::Default);
        assert!(!s.skip_sparse && !s.combine_sparse && !s.interface_features);
        assert!(!s.edge_interface);
        assert_eq!(s.filaments.len(), 1);
        let pla = &s.filaments[0];
        assert_eq!(pla.category, 100);
        assert_eq!((pla.prime_volume, pla.prime_volume_nc), (45.0, 60.0));
        assert_eq!(pla.iron_area, 4.0);
        assert!(pla.speed_factor_curve.is_empty());
        assert_eq!(s.prime_volume(0, false), 45.0);
        assert!(s.notes.is_empty());
    }

    #[test]
    fn each_filament_gets_its_own_values() {
        let s = read(
            r#"{"printer_model": "Bambu Lab P1S", "prime_volume": "20",
                "filament_type": ["PLA", "PETG", "PLA"],
                "filament_is_support": ["0", "0", "1"],
                "filament_diameter": ["1.75", "1.75", "1.75"],
                "filament_adhesiveness_category": ["0", "0", "0"],
                "filament_prime_volume": ["30", "40", "50"],
                "filament_prime_volume_nc": ["60", "70", "80"],
                "filament_tower_ironing_area": ["4", "6", "8"],
                "filament_cooling_before_tower": ["10", "nil", "0"],
                "filament_prime_tower_speed_factor_curve": ["", "0.5, 1,2.5x", "9"],
                "prime_tower_infill_gap": "165%",
                "prime_tower_enable_framework": "1",
                "prime_tower_flat_ironing": "1",
                "prime_tower_line_ironing": "0",
                "prime_tower_ironing_speed": "12",
                "bottom_solid_infill_flow_ratio": "0.95",
                "extruder_printable_height": ["250"],
                "prime_tower_line_width": "0.6"}"#,
        );
        let categories: Vec<i32> = s.filaments.iter().map(|f| f.category).collect();
        assert_eq!(categories, [100, 300, 702]);
        assert_eq!(s.prime_volume(1, false), 40.0);
        assert_eq!(s.prime_volume(2, true), 80.0);
        // Not a Bambu Lab printer: every change primes prime_volume.
        let other = Settings {
            bambu: false,
            ..s.clone()
        };
        assert_eq!(other.prime_volume(1, false), 20.0);
        assert_eq!(s.filaments[2].iron_area, 8.0);
        assert_eq!(s.filaments[0].cooling_before_tower, 10.0);
        assert_eq!(s.filaments[1].cooling_before_tower, 0.0);
        assert_eq!(s.filaments[1].speed_factor_curve, [0.5, 1.0, 2.5]);
        assert_eq!(s.filaments[2].speed_factor_curve, [5.0]);
        assert!((s.infill_gap - 1.65).abs() < 1e-6);
        assert!(s.framework && s.flat_ironing && !s.line_ironing);
        assert_eq!(s.ironing_speed, 720.0);
        assert_eq!(s.first_layer_flow_ratio, 0.95);
        assert_eq!(s.printable_height, [250.0]);
        assert_eq!(s.line_width, 0.6);
    }

    #[test]
    fn a_configured_category_wins_and_types_fall_back_to_bambus() {
        assert_eq!(adhesiveness_category(150, "PLA", false), 150);
        for (material, support, category) in [
            ("pla", false, 100),
            ("PLA-CF", false, 100),
            ("ASA", false, 200),
            ("PET-CF", false, 800),
            ("PETG", false, 300),
            ("PPS-CF", false, 801),
            ("PPA-CF", false, 400),
            ("PP", false, 902),
            ("PA6", false, 400),
            ("PC", false, 500),
            ("TPU", false, 600),
            ("PE", false, 901),
            ("PVA", true, 704),
            ("PLA", true, 702),
            ("PETG", true, 703),
            ("ABS", true, 706),
            ("TPU", true, 0),
            ("WOOD", false, 0),
        ] {
            assert_eq!(
                adhesiveness_category(0, material, support),
                category,
                "{material} {support}"
            );
        }
    }

    #[test]
    fn saving_primes_15_and_a_bad_curve_is_said() {
        let s = read(
            r#"{"prime_volume_mode": "Saving", "printer_model": "Bambu Lab A1",
                "filament_prime_tower_speed_factor_curve": ["fast"]}"#,
        );
        assert_eq!(s.prime_volume(0, false), 15.0);
        assert!(s.filaments[0].speed_factor_curve.is_empty());
        assert_eq!(s.notes.len(), 1, "{:?}", s.notes);
        assert!(
            Settings::from_settings(
                &Config::from_json(r#"{"prime_volume_mode": "Sometimes"}"#).unwrap(),
                0
            )
            .is_err()
        );
    }

    #[test]
    fn interface_features_count_only_where_the_reference_reads_them() {
        let on = |extra: &str| {
            read(&format!(
                r#"{{"enable_tower_interface_features": "1"{extra}}}"#
            ))
            .interface_features
        };
        // One nozzle (A1, P1S, X1C) and the H2S: nothing reads it.
        assert!(!on(r#", "nozzle_diameter": ["0.4"]"#));
        // Two nozzles (H2D) or rack hotends counted as extruders (H3C).
        assert!(on(r#", "nozzle_diameter": ["0.4", "0.4"]"#));
        assert!(on(r#", "handle_hotend_as_extruder": "1""#));
        // And the tower follows it where it acts: no note either way.
        let said = |nozzles: &str| {
            let json = format!(
                r#"{{"enable_prime_tower": "1", "printer_model": "Bambu Lab A1",
                    "enable_tower_interface_features": "1", "nozzle_diameter": {nozzles}}}"#
            );
            crate::TowerSettings::from_settings(&Config::from_json(&json).unwrap(), 0)
                .unwrap()
                .unsupported
                .iter()
                .any(|note| note.starts_with("enable_tower_interface_features"))
        };
        assert!(!said(r#"["0.4"]"#));
        assert!(!said(r#"["0.4", "0.4"]"#));
    }

    #[test]
    fn walls_bambu_names_and_sparse_layers() {
        let s = read(
            r#"{"prime_tower_rib_wall": "1", "prime_tower_rib_width": "6",
                "prime_tower_max_speed": "60", "wipe_tower_no_sparse_layers": "1",
                "wipe_tower_sparse_layers_combination": "1"}"#,
        );
        assert_eq!(
            s.wall,
            Wall::Ribs {
                width: 6.0,
                extra: 0.0,
                fillet: true
            }
        );
        assert_eq!(s.max_speed, 3600.0);
        // Leaving the layers out wins over laying them as one.
        assert!(s.skip_sparse && !s.combine_sparse);
        let smooth = read(r#"{"wipe_tower_no_sparse_layers": "1", "timelapse_type": "1"}"#);
        assert!(!smooth.skip_sparse && smooth.smooth_timelapse);
        // A cone is laid as a box by this tower.
        assert_eq!(read(r#"{"wipe_tower_wall_type": "cone"}"#).wall, Wall::Box);
    }
}
