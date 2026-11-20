//! Nozzles of two sizes on a Bambu Lab printer of two extruders (an H2D
//! with a 0.4 and a 0.6 nozzle): each nozzle prints with a process made
//! for its own size, as the reference's per-nozzle processes do
//! (NozzleProcess.cpp, PresetBundle.cpp `auto_nozzle_process_in`,
//! Snapmaker_Orca.cpp l.4040-4171).
//!
//! What the reference does, and this module with it:
//!
//! - The plate's process "claims" a nozzle size: the one size every
//!   printer in its `compatible_printers` is for (`printer_variant`,
//!   Preset.cpp `filament_preset_nozzle_diameter`). A nozzle of another
//!   size takes the process named for it (`nozzle_process_presets`), or
//!   else the system process of its size picked for it
//!   (`pick_nozzle_process`): of the plate's vendor, made for a printer of
//!   the same model, of the plate's quality ("Standard"), of the nearest
//!   layer height.
//! - From that process the nozzle takes its speeds, accelerations and jerk
//!   (Bambu's per-extruder slots, `print_extruder_id`), its line widths
//!   (per filament: `nozzle_*_line_width`) and every other region setting
//!   (`nozzle_region_values`: walls, infill, top and bottom shells,
//!   ironing...). Everything else (layer height, supports, tower, seams)
//!   stays the plate's.
//! - [`fill`] works all of that out into the settings, as the reference's
//!   command line fills its config before slicing; [`filament_overrides`]
//!   reads it back as what each filament's regions print with.
//!
//! Where this differs from the reference on purpose: a filament takes the
//! process of the nozzle `filament_map` puts it on. The reference takes
//! the nozzle of the filament's own number whenever there are no more
//! filaments than nozzles (PrintConfig.cpp l.635 `physical_extruder_for_filament`),
//! so on an H2D with two filaments mapped 2,1 or 2,2 a filament printed
//! from the 0.6 nozzle gets the 0.4 process's widths (measured: 0.03073
//! E/mm at 0.2 mm, a 0.42 mm line, from the 0.6 nozzle), while its speeds
//! do follow the map (`process_extruder_for_filament`). That is a bug and
//! is not copied.
//!
//! Refused here (an error, no G-code): an automatic filament map (which
//! nozzle a filament prints from is only settled after the walls are laid),
//! the project's own changes to a nozzle's process (`nozzle_process_edits`),
//! line widths per nozzle set by hand (`process_nozzle_line_widths`),
//! `nozzle_diameter_override`, a nozzle no process is made for, a process
//! without the per-extruder slot layout, and without the vendor profiles
//! to pick from. [`check_merging`] refuses a plate where the reference would
//! print the larger nozzle's layers merged into slabs (AdaptiveLayerMerge.cpp).

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use edge_config::{Kind, Settings, Value, Vendor};

use crate::Failure;

/// The reference's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// PrintConfig.cpp `per_nozzle_process_options`: `process_flow_variant_options`
/// but the four read once for the whole print.
pub(crate) const PER_NOZZLE_KEYS: [&str; 45] = [
    "outer_wall_speed",
    "inner_wall_speed",
    "sparse_infill_speed",
    "internal_solid_infill_speed",
    "top_surface_speed",
    "gap_infill_speed",
    "bridge_speed",
    "internal_bridge_speed",
    "ironing_speed",
    "small_perimeter_speed",
    "small_perimeter_threshold",
    "support_speed",
    "small_support_perimeter_speed",
    "small_support_perimeter_threshold",
    "support_interface_speed",
    "initial_layer_speed",
    "initial_layer_infill_speed",
    "initial_layer_travel_speed",
    "travel_speed",
    "overhang_1_4_speed",
    "overhang_2_4_speed",
    "overhang_3_4_speed",
    "overhang_4_4_speed",
    "enable_overhang_speed",
    "slowdown_for_curled_perimeters",
    "default_acceleration",
    "outer_wall_acceleration",
    "inner_wall_acceleration",
    "top_surface_acceleration",
    "initial_layer_acceleration",
    "bridge_acceleration",
    "travel_acceleration",
    "initial_layer_travel_acceleration",
    "sparse_infill_acceleration",
    "internal_solid_infill_acceleration",
    "accel_to_decel_enable",
    "accel_to_decel_factor",
    "default_jerk",
    "outer_wall_jerk",
    "inner_wall_jerk",
    "infill_jerk",
    "top_surface_jerk",
    "initial_layer_jerk",
    "travel_jerk",
    "default_junction_deviation",
];

fn per_nozzle_keys() -> impl Iterator<Item = &'static str> {
    PER_NOZZLE_KEYS.iter().copied()
}

/// NozzleProcess.cpp `nozzle_width_option_keys`.
const WIDTH_KEYS: [&str; 11] = [
    "line_width",
    "initial_layer_line_width",
    "outer_wall_line_width",
    "inner_wall_line_width",
    "top_surface_line_width",
    "sparse_infill_line_width",
    "internal_solid_infill_line_width",
    "support_line_width",
    "bridge_line_width",
    "skin_infill_line_width",
    "skeleton_infill_line_width",
];

/// NozzleProcess.cpp `nozzle_line_width_keys`, then the first layer's and
/// the general width (`fill_nozzle_line_widths`): each per-filament list
/// and the width it stands for.
pub(crate) const WIDTH_ARRAYS: [(&str, &str); 11] = [
    ("nozzle_outer_wall_line_width", "outer_wall_line_width"),
    ("nozzle_inner_wall_line_width", "inner_wall_line_width"),
    (
        "nozzle_sparse_infill_line_width",
        "sparse_infill_line_width",
    ),
    (
        "nozzle_internal_solid_infill_line_width",
        "internal_solid_infill_line_width",
    ),
    ("nozzle_top_surface_line_width", "top_surface_line_width"),
    ("nozzle_skin_infill_line_width", "skin_infill_line_width"),
    (
        "nozzle_skeleton_infill_line_width",
        "skeleton_infill_line_width",
    ),
    ("nozzle_bridge_line_width", "bridge_line_width"),
    ("nozzle_support_line_width", "support_line_width"),
    (
        "nozzle_initial_layer_line_width",
        "initial_layer_line_width",
    ),
    ("nozzle_line_width", "line_width"),
];

/// Region settings that are not process settings (`Preset::print_options`),
/// which a nozzle's process does not carry.
const NOT_PROCESS: [&str; 8] = [
    "filament_ironing_flow",
    "filament_ironing_inset",
    "filament_ironing_spacing",
    "filament_ironing_speed",
    "fuzzy_skin_skip_overhangs",
    "ironing_direction",
    "override_paint",
    "wall_infill_order",
];

/// NozzleProcess.cpp `is_process_meta_key`.
const META: [&str; 14] = [
    "inherits",
    "compatible_printers",
    "compatible_printers_condition",
    "print_settings_id",
    "print_extruder_id",
    "print_extruder_variant",
    "print_extruder_nozzle_diameter",
    "process_extruder_values",
    "process_flow_support",
    "version",
    "setting_id",
    "from",
    "name",
    "print_compatible_printers",
];

/// NozzleProcess.cpp `nozzle_region_option_keys`: the region settings a
/// nozzle's process gives besides its speeds and widths.
pub(crate) fn region_keys() -> impl Iterator<Item = &'static str> {
    crate::regions::REGION_KEYS.iter().copied().filter(|key| {
        !NOT_PROCESS.contains(key)
            && !crate::regions::FEATURE_FILAMENTS.contains(key)
            && !key.starts_with("nozzle_")
            && !PER_NOZZLE_KEYS.contains(key)
            && !WIDTH_KEYS.contains(key)
            && !META.contains(key)
    })
}

/// NozzleProcess.cpp `is_per_tool_process_option`.
fn per_tool(key: &str) -> bool {
    PER_NOZZLE_KEYS.contains(&key)
        || WIDTH_KEYS.contains(&key)
        || key == "line_width_nozzle_diameter"
        || key == "process_nozzle_line_widths"
        || region_keys().any(|region| region == key)
}

/// A value as one text, a list's entries joined by commas (as a region's
/// own settings hold it, `crate::regions::Regions::key`).
fn joined(value: &Value) -> String {
    match value {
        Value::One(text) => text.clone(),
        Value::Many(items) => items.join(","),
    }
}

/// Whether two values are the same: entry by entry, as text or as numbers.
fn same(a: &Value, b: &Value) -> bool {
    let entries = |value: &Value| -> Vec<String> {
        match value {
            Value::One(text) => vec![text.clone()],
            Value::Many(items) => items.clone(),
        }
    };
    let (a, b) = (entries(a), entries(b));
    a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| same_text(a, b))
}

fn same_text(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    if a == b {
        return true;
    }
    let number = |text: &str| -> Option<(f64, bool)> {
        match text.strip_suffix('%') {
            Some(percent) => percent.trim().parse().ok().map(|v| (v, true)),
            None => text.parse().ok().map(|v| (v, false)),
        }
    };
    matches!((number(a), number(b)), (Some(x), Some(y)) if x == y)
}

/// A nozzle diameter as the reference names it in messages ("0.6").
fn size_text(diameter: f64) -> String {
    let text = format!("{diameter:.2}");
    let text = text.trim_end_matches('0');
    text.trim_end_matches('.').to_owned()
}

/// The diameters of the nozzles, when the printer is one this is done for
/// and they differ: two extruders, not one nozzle fed by many filaments,
/// a printer that groups filaments onto its nozzles (the H2D).
pub(crate) fn mixed(settings: &Settings) -> Result<Option<Vec<f64>>, Failure> {
    let diameters: Vec<f64> = (settings.floats("nozzle_diameter")?.unwrap_or_default())
        .into_iter()
        .map(|diameter| diameter.unwrap_or(0.0))
        .collect();
    let mixed = diameters.len() == 2
        && (diameters[0] - diameters[1]).abs() > 1e-6
        && settings.flag("single_extruder_multi_material")? != Some(true)
        && crate::pipeline::groups_onto_nozzles(settings);
    Ok(mixed.then_some(diameters))
}

/// The extruder each filament prints from, from 0: the one `filament_map`
/// puts it on (see the module's notes on the reference).
pub(crate) fn extruders_of(settings: &Settings, extruders: usize) -> Vec<usize> {
    let count = settings
        .texts("filament_diameter")
        .map_or(0, <[String]>::len);
    let map = settings.texts("filament_map").unwrap_or_default();
    (0..count)
        .map(|filament| {
            match map
                .get(filament)
                .and_then(|entry| entry.trim().parse::<usize>().ok())
            {
                Some(extruder) if (1..=extruders).contains(&extruder) => extruder - 1,
                // Not mapped yet: the reference's best effort.
                _ if filament < extruders => filament,
                _ => 0,
            }
        })
        .collect()
}

/// A flow mode of a process slot (`FilamentVolumeType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Standard,
    HighFlow,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::HighFlow => "high_flow",
        }
    }
}

/// The nozzle part of a slot's variant, "Standard" of "Direct Drive Standard".
fn nozzle_part(variant: &str) -> &str {
    ["Direct Drive ", "Bowden "]
        .iter()
        .find_map(|prefix| variant.strip_prefix(prefix))
        .filter(|rest| !rest.is_empty())
        .unwrap_or("")
}

/// PrintConfig.cpp `process_variant_flow_mode`.
fn flow_mode(variant: &str) -> Option<Mode> {
    match nozzle_part(variant) {
        "Standard" => Some(Mode::Standard),
        "High Flow" => Some(Mode::HighFlow),
        _ => None,
    }
}

/// PrintConfig.cpp `process_variant_fill_mode`.
fn fill_mode(variant: &str) -> Mode {
    if let Some(mode) = flow_mode(variant) {
        return mode;
    }
    let part = nozzle_part(variant);
    if part.contains("TPU") || !part.contains("High Flow") {
        Mode::Standard
    } else {
        Mode::HighFlow
    }
}

/// A process's per-extruder slot layout (NozzleProcess.cpp `SlotLayout`).
#[derive(Clone, Debug, Default)]
struct Layout {
    ids: Vec<usize>,
    variants: Vec<String>,
    flow_support: Vec<String>,
}

impl Layout {
    fn of(settings: &Settings) -> Self {
        let texts = |key: &str| {
            settings
                .texts(key)
                .map(<[String]>::to_vec)
                .unwrap_or_default()
        };
        Self {
            ids: texts("print_extruder_id")
                .iter()
                .map(|id| id.trim().parse().unwrap_or(0))
                .collect(),
            variants: texts("print_extruder_variant"),
            flow_support: texts("process_flow_support"),
        }
    }

    /// The extruders of a valid layout (ids 1, 1, .., 2, .., n, n >= 2), else 0.
    fn extruder_count(&self) -> usize {
        if self.ids.is_empty() || self.ids.len() != self.variants.len() {
            return 0;
        }
        let mut count = 0;
        for id in &self.ids {
            if *id == count + 1 {
                count += 1;
            } else if count == 0 || *id != count {
                return 0;
            }
        }
        if count >= 2 { count } else { 0 }
    }

    /// The column of `mode` (`process_flow_support`, Standard when it is
    /// not declared): what every extruder reads without per-extruder values.
    fn column(&self, size: usize, mode: Mode) -> usize {
        let index = (self.flow_support.iter())
            .position(|declared| declared == mode.name())
            .unwrap_or(0);
        if index < size { index } else { 0 }
    }

    /// The entry of a list of `size` holding extruder `extruder`'s value
    /// for `mode`.
    fn index(&self, size: usize, extruder: usize, mode: Mode) -> usize {
        if size == 0 {
            return 0;
        }
        if size == self.ids.len() && extruder < self.extruder_count() {
            let mut first = None;
            for (slot, id) in self.ids.iter().enumerate() {
                if *id != extruder + 1 {
                    continue;
                }
                first.get_or_insert(slot);
                if flow_mode(&self.variants[slot]) == Some(mode) {
                    return slot;
                }
            }
            if let Some(first) = first {
                return first;
            }
        }
        self.column(size, mode)
    }
}

/// The flow mode each filament prints at (`filament_volume_type`).
fn filament_modes(settings: &Settings, count: usize) -> Vec<Mode> {
    let kinds = settings.texts("filament_volume_type").unwrap_or_default();
    (0..count)
        .map(
            |filament| match kinds.get(filament).map(|kind| kind.trim()) {
                Some("high_flow") => Mode::HighFlow,
                _ => Mode::Standard,
            },
        )
        .collect()
}

/// PrintConfig.cpp `process_config_idx` with `process_extruder_values` on:
/// the entry of a per-nozzle list of `size` filament `filament` (from 0)
/// reads, on extruder `extruder`.
fn slot_of(layout: &Layout, size: usize, extruder: usize, mode: Mode) -> usize {
    let legacy = layout.column(size, mode);
    if size < 2 || size != layout.ids.len() || layout.extruder_count() < 2 {
        return legacy;
    }
    if layout.ids[legacy] == extruder + 1 {
        return legacy;
    }
    let wanted = flow_mode(&layout.variants[legacy]);
    let (mut first, mut named) = (None, None);
    for (slot, id) in layout.ids.iter().enumerate() {
        if *id != extruder + 1 {
            continue;
        }
        first.get_or_insert(slot);
        if wanted.is_some() && flow_mode(&layout.variants[slot]) == wanted {
            return slot;
        }
        if named.is_none() && layout.variants[slot] == layout.variants[legacy] {
            named = Some(slot);
        }
    }
    named.or(first).unwrap_or(legacy)
}

/// One vendor's presets, each read once.
struct Bundle {
    vendor: Vendor,
    machines: HashMap<String, Option<Settings>>,
}

impl Bundle {
    fn machine(&mut self, name: &str) -> Option<&Settings> {
        let vendor = &self.vendor;
        self.machines
            .entry(name.to_owned())
            .or_insert_with(|| vendor.resolve(Kind::Machine, name).ok())
            .as_ref()
    }

    /// Preset.cpp `filament_preset_nozzle_diameter`: the one nozzle size
    /// every printer in a preset's `compatible_printers` is for, 0 where
    /// they span sizes or none says one.
    fn claim(&mut self, preset: &Settings) -> f64 {
        let printers = preset
            .texts("compatible_printers")
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let mut found = 0.0;
        for printer in &printers {
            let Some(variant) = self
                .machine(printer)
                .and_then(|machine| machine.text("printer_variant"))
            else {
                continue;
            };
            let Ok(size) = variant.parse::<f64>() else {
                continue;
            };
            if size <= 0.0 {
                continue;
            }
            if found == 0.0 {
                found = size;
            } else if (found - size).abs() > EPSILON {
                return 0.0;
            }
        }
        found
    }

    /// Whether one of a preset's printers is of `model`.
    fn for_model(&mut self, preset: &Settings, model: &str) -> bool {
        let printers = preset
            .texts("compatible_printers")
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        !model.is_empty()
            && printers.iter().any(|printer| {
                self.machine(printer)
                    .and_then(|machine| machine.text("printer_model"))
                    == Some(model)
            })
    }
}

/// "Standard" of "0.20mm Standard @BBL H2D" (NozzleProcess.cpp `process_quality_word`).
fn quality_word(name: &str) -> &str {
    match (name.find("mm "), name.find(" @")) {
        (Some(mm), Some(at)) if at > mm + 3 => &name[mm + 3..at],
        _ => "",
    }
}

/// NozzleProcess.cpp `pick_nozzle_process`, over system presets.
fn pick(candidates: &[(String, f64)], plate_layer_height: f64, plate_name: &str) -> Option<String> {
    let word = quality_word(plate_name);
    candidates
        .iter()
        .min_by(|(a, a_height), (b, b_height)| {
            let rank = |name: &str, height: f64| {
                (
                    usize::from(word.is_empty() || quality_word(name) != word),
                    ((height - plate_layer_height).abs() * 1000.0).round() as i64,
                    usize::from(quality_word(name) != "Standard"),
                )
            };
            rank(a, *a_height)
                .cmp(&rank(b, *b_height))
                .then_with(|| a.cmp(b))
        })
        .map(|(name, _)| name.clone())
}

/// The tool's name in messages: "Left (T1)", "Right (T2)" on Bambu's
/// two-extruder machines (NozzleProcess.cpp `nozzle_tool_name`).
fn tool_name(extruder: usize) -> String {
    format!(
        "{} (T{})",
        if extruder == 0 { "Left" } else { "Right" },
        extruder + 1
    )
}

/// A width as a list holds it: as given, or a number worked out.
fn width_text(value: f64, percent: bool) -> String {
    if percent {
        format!("{value}%")
    } else {
        format!("{value}")
    }
}

fn parse_width(text: &str) -> Option<(f64, bool)> {
    let text = text.trim();
    match text.strip_suffix('%') {
        Some(percent) => percent.trim().parse().ok().map(|v| (v, true)),
        None => text.parse().ok().map(|v| (v, false)),
    }
}

/// A 0 of these widths means "line_width"; a 0 bridge width means auto and a
/// 0 first-layer width "the role widths".
fn zero_is_line_width(key: &str) -> bool {
    !matches!(
        key,
        "bridge_line_width" | "initial_layer_line_width" | "line_width"
    )
}

/// NozzleProcess.cpp `line_width_reference_diameter`.
fn reference_diameter(settings: &Settings, diameters: &[f64]) -> f64 {
    if let Ok(Some(own)) = settings.float("line_width_nozzle_diameter")
        && own > 0.0
    {
        return own;
    }
    if let Some(variant) = settings.text("printer_variant")
        && let Some(start) = variant.find(|c: char| c.is_ascii_digit() || c == '.')
    {
        let rest = &variant[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if let Ok(size) = rest[..end].parse::<f64>()
            && size > 0.05
            && size < 3.0
        {
            return size;
        }
    }
    diameters.first().copied().unwrap_or(0.0)
}

/// NozzleProcess.cpp `nozzle_width_for`, rules 3 and 4 (the per-nozzle
/// table is refused before this): the plate's width of `key` on a nozzle of
/// `diameter`, an absolute one scaled from the size the process is made for.
/// Second: whether it was scaled.
fn plate_width_for(
    settings: &Settings,
    key: &str,
    diameter: f64,
    diameters: &[f64],
) -> (String, bool) {
    let plate = settings
        .texts(key)
        .and_then(|texts| texts.first())
        .cloned()
        .unwrap_or_else(|| "0".to_owned());
    let Some((value, percent)) = parse_width(&plate) else {
        return (plate, false);
    };
    if value == 0.0 && zero_is_line_width(key) {
        let (line, scaled) = plate_width_for(settings, "line_width", diameter, diameters);
        return if scaled { (line, true) } else { (plate, false) };
    }
    if percent || value <= 0.0 || diameter <= 0.0 {
        return (plate, false);
    }
    let reference = reference_diameter(settings, diameters);
    if reference <= 0.0 || (reference - diameter).abs() < 0.001 {
        return (plate, false);
    }
    let scaled = (value * diameter / reference * 10000.0).round() / 10000.0;
    (width_text(scaled, false), true)
}

/// Whether a vendor's index names `preset` at all: read as text, so the
/// vendors that do not are not opened (opening one reads the filament
/// library's index too).
fn lists(profiles: &Path, vendor: &str, preset: &str) -> bool {
    std::fs::read_to_string(profiles.join(format!("{vendor}.json")))
        .is_ok_and(|index| index.contains(&format!("\"{preset}\"")))
}

/// Each nozzle's process, as far as it is a nozzle's own: its settings
/// laid over the reference's defaults, or none where the nozzle prints
/// with the plate's.
type Sources = Vec<Option<Settings>>;

/// Works out each nozzle's process and fills the settings with what each
/// nozzle and filament prints with (NozzleProcess.cpp
/// `fill_nozzle_process_slots`, `resolve_nozzle_line_widths`). Does
/// nothing unless the nozzles differ in size ([`mixed`]). Returns notes.
pub(crate) fn fill(
    settings: &mut Settings,
    profiles: Option<&Path>,
) -> Result<Vec<String>, Failure> {
    let Some(diameters) = mixed(settings)? else {
        return Ok(Vec::new());
    };
    let sizes = format!(
        "nozzles of two sizes ({} and {} mm)",
        size_text(diameters[0]),
        size_text(diameters[1])
    );
    let refuse =
        |what: &str| -> Failure { format!("{sizes}: {what}; no G-code was written").into() };
    let mode = settings
        .text("filament_map_mode")
        .unwrap_or("Auto For Flush");
    if mode != "Manual" {
        return Err(refuse(&format!(
            "with filament_map_mode \"{mode}\" which nozzle each filament prints from, and so \
             the process its walls and fill are laid with, is only settled after they are laid; \
             set the filaments' nozzles by hand (Manual)"
        )));
    }
    let blank = |key: &str| {
        settings.texts(key).is_none_or(|texts| {
            texts
                .iter()
                .all(|text| text.trim().is_empty() || text.trim() == "0")
        })
    };
    for (key, what) in [
        (
            "nozzle_process_edits",
            "a nozzle's own changes to its process",
        ),
        ("process_nozzle_line_widths", "line widths set per nozzle"),
        (
            "nozzle_diameter_override",
            "a nozzle size set in the project",
        ),
        (
            "nozzle_slot_process_presets",
            "processes for a nozzle rack's sizes",
        ),
    ] {
        if !blank(key) {
            return Err(refuse(&format!("{key} ({what}) is not followed yet")));
        }
    }
    // Support printed in a filament of its own takes that filament's
    // nozzle's width for its lines (PrintObject.cpp l.4510, Flow.cpp l.265);
    // not done here.
    let support = settings.flag("enable_support")? == Some(true)
        || settings.integer("raft_layers")?.unwrap_or(0) > 0;
    for key in ["support_filament", "support_interface_filament"] {
        if support && settings.integer(key)?.unwrap_or(0) > 0 {
            return Err(refuse(&format!(
                "support printed in a filament of its own ({key}) is not laid at its nozzle's \
                 line width yet; leave it at the default (0)"
            )));
        }
    }
    let Some(profiles) = profiles else {
        return Err(refuse(
            "each nozzle prints with a process of its own size, picked from the vendor's \
             profiles, and none were given (--profiles or EDGE_PROFILES)",
        ));
    };
    let plate_name = settings
        .text("print_settings_id")
        .unwrap_or_default()
        .to_owned();
    let plate_layer_height = settings.float("layer_height")?.unwrap_or(0.0);
    let model = settings
        .text("printer_model")
        .unwrap_or_default()
        .to_owned();
    // The plate's process among the vendors' system presets, and the
    // vendor to pick other sizes' processes from.
    let mut bundle: Option<Bundle> = None;
    let mut claim = 0.0;
    for name in Vendor::list(profiles)? {
        if plate_name.is_empty() || !lists(profiles, &name, &plate_name) {
            continue;
        }
        let vendor = Vendor::open(profiles, &name)?;
        if vendor.has(Kind::Process, &plate_name) {
            let preset = vendor.resolve(Kind::Process, &plate_name)?;
            let mut found = Bundle {
                vendor,
                machines: HashMap::new(),
            };
            claim = found.claim(&preset);
            bundle = Some(found);
            break;
        }
    }
    let mut notes = Vec::new();
    let names: Vec<String> = (0..2)
        .map(|extruder| {
            settings
                .texts("nozzle_process_presets")
                .and_then(|names| names.get(extruder))
                .map(|name| name.trim().to_owned())
                .unwrap_or_default()
        })
        .collect();
    let defaults = edge_config::reference_defaults();
    let mut sources: Sources = vec![None, None];
    for (extruder, diameter) in diameters.iter().enumerate() {
        let chosen = !names[extruder].is_empty();
        let mut name = names[extruder].clone();
        if name.is_empty()
            && claim > 0.0
            && (claim - diameter).abs() > EPSILON
            && let Some(bundle) = &mut bundle
        {
            let mut candidates = Vec::new();
            // Every process of the vendor read side by side (a few hundred
            // files and their parents).
            let all: Vec<(String, Settings)> = {
                use rayon::prelude::*;
                let names: Vec<&str> = bundle.vendor.names(Kind::Process).collect();
                let vendor = &bundle.vendor;
                names
                    .par_iter()
                    .filter_map(|name| {
                        let preset = vendor.resolve(Kind::Process, name).ok()?;
                        Some(((*name).to_owned(), preset))
                    })
                    .collect()
            };
            for (candidate, preset) in all {
                if preset.text("instantiation") != Some("true") {
                    continue;
                }
                let own = bundle.claim(&preset);
                if own <= 0.0
                    || (own - diameter).abs() > EPSILON
                    || !bundle.for_model(&preset, &model)
                {
                    continue;
                }
                let height = preset.float("layer_height").ok().flatten().unwrap_or(0.0);
                candidates.push((candidate, height));
            }
            let base = if plate_name.is_empty() {
                ""
            } else {
                &plate_name
            };
            name = pick(&candidates, plate_layer_height, base).ok_or_else(|| {
                refuse(&format!(
                    "{tool} has a {size} mm nozzle, and neither the plate's process nor any \
                     process of this printer is made for {size} mm nozzles",
                    tool = tool_name(extruder),
                    size = size_text(*diameter)
                ))
            })?;
            notes.push(format!(
                "per-nozzle process: T{} ({} mm) uses '{name}', picked for its size",
                extruder + 1,
                size_text(*diameter)
            ));
        }
        if name.is_empty() {
            continue;
        }
        // A named process is one of the vendors' system presets (the
        // reference also looks among the person's own, which a command line
        // has none of).
        let mut base = match &bundle {
            Some(bundle) if bundle.vendor.has(Kind::Process, &name) => {
                Some(bundle.vendor.resolve(Kind::Process, &name)?)
            }
            _ => None,
        };
        if base.is_none() {
            for vendor in Vendor::list(profiles)? {
                if !lists(profiles, &vendor, &name) {
                    continue;
                }
                let vendor = Vendor::open(profiles, &vendor)?;
                if vendor.has(Kind::Process, &name) {
                    base = Some(vendor.resolve(Kind::Process, &name)?);
                    break;
                }
            }
        }
        let Some(base) = base else {
            return Err(refuse(&format!(
                "the process \"{name}\" named for T{} is not one of the vendors' presets",
                extruder + 1
            )));
        };
        if chosen
            && let Some(own) = base.float("layer_height")?
            && (own - plate_layer_height).abs() > EPSILON
        {
            return Err(refuse(&format!(
                "the process \"{name}\" named for T{} has a layer height of its own ({own} mm), \
                 which the reference prints as that nozzle's layers merged into slabs; that is \
                 not done",
                extruder + 1
            )));
        }
        let mut tool = defaults.clone();
        tool.overlay(&base);
        let differs = tool.iter().any(|(key, value)| {
            per_tool(key) && settings.get(key).is_none_or(|plate| !same(value, plate))
        });
        if differs {
            sources[extruder] = Some(tool);
        }
    }
    let filaments = extruders_of(settings, 2);
    if sources.iter().all(Option::is_none) {
        // No nozzle has a process of its own: the plate's widths, scaled
        // for a nozzle of another size (Print::apply's
        // `resolve_nozzle_line_widths`), and only when one changes.
        let arrays = width_arrays(settings, &sources, &filaments, &diameters);
        let changed = arrays
            .iter()
            .zip(WIDTH_ARRAYS)
            .any(|((_, values), (_, key))| {
                let plate = settings
                    .texts(key)
                    .and_then(|texts| texts.first())
                    .map_or("0", String::as_str);
                values.iter().any(|value| !same_text(value, plate))
            });
        if changed {
            for (array, values) in arrays {
                settings.insert(array, Value::Many(values));
            }
        }
        return Ok(notes);
    }
    fill_slots(settings, &sources).map_err(|what| refuse(&what))?;
    for (array, values) in width_arrays(settings, &sources, &filaments, &diameters) {
        settings.insert(array, Value::Many(values));
    }
    let regions = region_values(settings, &sources, &filaments);
    settings.insert("nozzle_region_values", Value::Many(regions));
    let differ = slots_differ(settings);
    let split = settings.flag("process_extruder_values")? == Some(true);
    let named = names.iter().any(|name| !name.is_empty());
    if split || differ || named {
        settings.insert(
            "process_extruder_values",
            Value::One(if split || differ { "1" } else { "0" }.to_owned()),
        );
        if settings.get("nozzle_process_presets").is_none() {
            settings.insert("nozzle_process_presets", Value::Many(Vec::new()));
        }
    }
    Ok(notes)
}

/// NozzleProcess.cpp `fill_nozzle_line_widths`: per filament, each width
/// of the process of the nozzle it prints from.
fn width_arrays(
    settings: &Settings,
    sources: &Sources,
    filaments: &[usize],
    diameters: &[f64],
) -> Vec<(String, Vec<String>)> {
    WIDTH_ARRAYS
        .iter()
        .map(|(array, key)| {
            let values = filaments
                .iter()
                .map(|extruder| {
                    let diameter = diameters[*extruder];
                    let Some(source) = &sources[*extruder] else {
                        return plate_width_for(settings, key, diameter, diameters).0;
                    };
                    let own =
                        |key: &str| source.texts(key).and_then(|texts| texts.first()).cloned();
                    let plate = settings
                        .texts(key)
                        .and_then(|texts| texts.first())
                        .cloned()
                        .unwrap_or_else(|| "0".to_owned());
                    let width = own(key);
                    if !zero_is_line_width(key) {
                        return width.unwrap_or(plate);
                    }
                    let value = width.as_deref().and_then(parse_width);
                    let width = match value {
                        Some((v, _)) if v != 0.0 => width,
                        _ => own("line_width"),
                    };
                    match width.as_deref().and_then(parse_width) {
                        Some((v, _)) if v > 0.0 => width.unwrap_or(plate),
                        _ => plate,
                    }
                })
                .collect();
            ((*array).to_owned(), values)
        })
        .collect()
}

/// NozzleProcess.cpp `fill_nozzle_region_values`: the region settings some
/// nozzle's process sets otherwise than the plate, per filament as
/// "key = value" lines.
fn region_values(settings: &Settings, sources: &Sources, filaments: &[usize]) -> Vec<String> {
    let keys: Vec<&str> = region_keys()
        .filter(|key| {
            settings.get(key).is_some_and(|plate| {
                sources
                    .iter()
                    .flatten()
                    .any(|source| source.get(key).is_some_and(|own| !same(own, plate)))
            })
        })
        .collect();
    if keys.is_empty() {
        return Vec::new();
    }
    filaments
        .iter()
        .map(|extruder| {
            keys.iter()
                .map(|key| {
                    let value = sources[*extruder]
                        .as_ref()
                        .and_then(|source| source.get(key))
                        .or_else(|| settings.get(key))
                        .map(joined)
                        .unwrap_or_default();
                    format!("{key} = {value}")
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect()
}

/// NozzleProcess.cpp `fill_nozzle_process_slots`, the speeds part: every
/// per-nozzle list laid out per extruder slot, a nozzle with a process of
/// its own taking that process's values.
fn fill_slots(settings: &mut Settings, sources: &Sources) -> Result<(), String> {
    let plate = Layout::of(settings);
    if plate.extruder_count() != 2 {
        return Err(
            "the process has no per-extruder slots (print_extruder_id, print_extruder_variant) \
             for two extruders, which per-nozzle processes are kept in; that is not done yet"
                .to_owned(),
        );
    }
    let split = settings.flag("process_extruder_values").ok().flatten() == Some(true);
    let layout = plate.ids.len();
    let source_layouts: Vec<Option<Layout>> = sources
        .iter()
        .map(|source| source.as_ref().map(Layout::of))
        .collect();
    for key in per_nozzle_keys() {
        let Some(plate_values) = settings.texts(key).map(<[String]>::to_vec) else {
            continue;
        };
        if plate_values.is_empty() {
            continue;
        }
        let in_place = plate_values.len() == layout;
        let mut values = Vec::with_capacity(layout);
        for slot in 0..layout {
            let extruder = plate.ids[slot] - 1;
            let mode = fill_mode(&plate.variants[slot]);
            if let (Some(Some(source)), Some(Some(own_layout))) =
                (sources.get(extruder), source_layouts.get(extruder))
                && let Some(own) = source.texts(key).filter(|own| !own.is_empty())
            {
                values.push(own[own_layout.index(own.len(), extruder, mode)].clone());
                continue;
            }
            let index = if !in_place {
                plate.index(plate_values.len(), extruder, mode)
            } else if split || extruder == 0 {
                slot
            } else {
                plate.column(plate_values.len(), mode)
            };
            values.push(plate_values[index].clone());
        }
        settings.insert(key, Value::Many(values));
    }
    Ok(())
}

/// PrintConfig.cpp `process_extruder_slots_differ`: whether an extruder's
/// Standard or High Flow slot holds another value than extruder 1's.
fn slots_differ(settings: &Settings) -> bool {
    let layout = Layout::of(settings);
    if layout.extruder_count() < 2 {
        return false;
    }
    let mut pairs = Vec::new();
    for (slot, id) in layout.ids.iter().enumerate() {
        let Some(mode) = flow_mode(&layout.variants[slot]).filter(|_| *id > 1) else {
            continue;
        };
        if let Some(first) = (0..layout.ids.len())
            .find(|own| layout.ids[*own] == 1 && flow_mode(&layout.variants[*own]) == Some(mode))
        {
            pairs.push((slot, first));
        }
    }
    per_nozzle_keys().any(|key| {
        settings.texts(key).is_some_and(|values| {
            values.len() == layout.ids.len()
                && pairs
                    .iter()
                    .any(|(slot, first)| !same_text(&values[*slot], &values[*first]))
        })
    })
}

/// What each filament's regions print with over the plate's settings,
/// from what [`fill`] worked out (NozzleProcess.cpp `stamp_nozzle_line_widths`,
/// `stamp_nozzle_region_values`, PrintRegion.cpp's first-layer width and the
/// speeds of `process_config_idx`): its widths, its nozzle's size, its
/// process's region settings and speeds, each only where it differs from
/// what the plate's settings give that filament. None where nothing was
/// filled. A per-nozzle setting the engine reads for the whole plate and
/// not per region that would differ per filament is refused.
pub(crate) fn filament_overrides(
    settings: &Settings,
) -> Result<Option<Vec<BTreeMap<String, String>>>, Failure> {
    let Some(diameters) = mixed(settings)? else {
        return Ok(None);
    };
    let count = settings
        .texts("filament_diameter")
        .map_or(0, <[String]>::len);
    let filled = |array: &str| {
        settings
            .texts(array)
            .is_some_and(|values| values.len() == count)
    };
    if count == 0 || !filled("nozzle_outer_wall_line_width") {
        return Ok(None);
    }
    let extruders = extruders_of(settings, 2);
    let modes = filament_modes(settings, count);
    let layout = Layout::of(settings);
    let split = settings.flag("process_extruder_values")? == Some(true);
    let regions = settings.texts("nozzle_region_values").unwrap_or_default();
    let mut unfollowed = Vec::new();
    let mut out = Vec::with_capacity(count);
    for filament in 0..count {
        let extruder = extruders[filament];
        let mut own = BTreeMap::new();
        for (array, key) in WIDTH_ARRAYS {
            // Support is laid with the object's width by the support's
            // own filament (`support_filament`), not a region's.
            if key == "support_line_width" {
                continue;
            }
            let Some(value) = settings
                .texts(array)
                .and_then(|values| values.get(filament))
            else {
                continue;
            };
            let plate = settings
                .texts(key)
                .and_then(|texts| texts.first())
                .map_or("0", String::as_str);
            if !same_text(value, plate) {
                own.insert(key.to_owned(), value.clone());
            }
        }
        // Flow, bridges and widths given as a share are of the nozzle the
        // filament prints from; every stage reads the first nozzle's size.
        if (diameters[extruder] - diameters[0]).abs() > 1e-9 {
            own.insert("nozzle_diameter".to_owned(), size_text(diameters[extruder]));
        }
        if let Some(entry) = regions.get(filament) {
            for line in entry.lines() {
                let Some((key, value)) = line.split_once(" = ") else {
                    continue;
                };
                let differs = settings
                    .get(key)
                    .is_none_or(|plate| !same_text(&joined(plate), value));
                if differs {
                    own.insert(key.to_owned(), value.to_owned());
                }
            }
        }
        if split {
            for key in per_nozzle_keys() {
                let Some(values) = settings.texts(key) else {
                    continue;
                };
                let value = &values[slot_of(&layout, values.len(), extruder, modes[filament])];
                // What the G-code writer reads for the filament otherwise
                // (`edge_gcode::PrintSettings::for_filament`).
                let read = values
                    .get(filament)
                    .or(values.last())
                    .map_or("", String::as_str);
                if same_text(value, read) {
                    continue;
                }
                if crate::regions::per_region(key) {
                    own.insert(key.to_owned(), value.clone());
                } else if !unfollowed.contains(&key) {
                    unfollowed.push(key);
                }
            }
        }
        out.push(own);
    }
    if !unfollowed.is_empty() {
        return Err(format!(
            "nozzles of two sizes: {} differ from nozzle to nozzle, and are not followed per \
             nozzle yet; no G-code was written",
            unfollowed.join(", ")
        )
        .into());
    }
    Ok(Some(out))
}

/// AdaptiveLayerMerge.cpp `adaptive_layer_band_size`: how many layers of
/// `base` a nozzle of `diameter` lays as one next to the finest of
/// `finest`, within `[min, max]`.
fn band_size(diameter: f64, finest: f64, base: f64, min: f64, max: f64) -> usize {
    if diameter <= EPSILON || finest <= EPSILON || base <= EPSILON || diameter <= finest + EPSILON {
        return 1;
    }
    let mut size = (diameter / finest + 0.5).floor().max(1.0) as usize;
    while (size as f64) * base + EPSILON < min {
        size += 1;
    }
    while size > 1 && (size as f64) * base > max + EPSILON {
        size -= 1;
    }
    if (size as f64) * base + EPSILON < min {
        1
    } else {
        size
    }
}

/// Refuses a plate whose filaments print from nozzles of two sizes where
/// the reference would lay the larger nozzle's walls as slabs of several
/// layers (PrintObject.cpp `compute_adaptive_merge_groups`), which is not
/// done: `walls` are the filaments (from 1) that lay walls, `used` every
/// filament that prints.
pub(crate) fn check_merging(
    settings: &Settings,
    walls: &[usize],
    used: &[usize],
    layer_height: f64,
) -> Result<(), Failure> {
    let Some(diameters) = mixed(settings)? else {
        return Ok(());
    };
    let extruders = extruders_of(settings, 2);
    let size_of = |filament: usize| {
        extruders
            .get(filament.wrapping_sub(1))
            .map_or(diameters[0], |extruder| diameters[*extruder])
    };
    // Print.cpp `uses_multiple_nozzle_diameters`: the plate prints from
    // nozzles of two sizes at all.
    let sizes: Vec<f64> = used.iter().map(|filament| size_of(*filament)).collect();
    if sizes.iter().all(|size| (size - sizes[0]).abs() <= EPSILON) {
        return Ok(());
    }
    let finest = walls
        .iter()
        .map(|filament| size_of(*filament))
        .fold(f64::INFINITY, f64::min);
    if !finest.is_finite() {
        return Ok(());
    }
    let limits = |key: &str| -> Vec<f64> {
        (settings.floats(key).ok().flatten().unwrap_or_default())
            .into_iter()
            .map(|value| value.unwrap_or(0.0))
            .collect()
    };
    let (mins, maxs) = (limits("min_layer_height"), limits("max_layer_height"));
    for filament in walls {
        let extruder = (filament.checked_sub(1))
            .and_then(|index| extruders.get(index))
            .copied()
            .unwrap_or(0);
        let diameter = diameters[extruder];
        let (mut min, mut max) = (0.2 * diameter, 0.7 * diameter);
        // The extruder's own limits, unless they leave no window.
        let (window_min, window_max) = (min, max);
        if let Some(own) = mins.get(extruder).filter(|own| **own > EPSILON) {
            min = min.max(*own);
        }
        if let Some(own) = maxs.get(extruder).filter(|own| **own > EPSILON) {
            max = max.min(*own);
        }
        if min > max + EPSILON {
            (min, max) = (window_min, window_max);
        }
        let band = band_size(diameter, finest, layer_height, min, max);
        if band >= 2 {
            return Err(format!(
                "nozzles of two sizes: filament {filament} prints from the {} mm nozzle, whose \
                 layers the reference lays {band} at a time as slabs of {:.2} mm next to the {} \
                 mm nozzle's; merging layers into slabs is not done yet; no G-code was written",
                size_text(diameter),
                band as f64 * layer_height,
                size_text(finest)
            )
            .into());
        }
    }
    Ok(())
}

/// The diameter of the nozzle each filament prints from, on a printer
/// whose nozzles differ in size; none on any other.
fn filament_sizes(settings: &Settings) -> Result<Option<Vec<f64>>, Failure> {
    let Some(diameters) = mixed(settings)? else {
        return Ok(None);
    };
    Ok(Some(
        (extruders_of(settings, 2).into_iter())
            .map(|extruder| diameters[extruder])
            .collect(),
    ))
}

/// Refuses the prime tower where the plate's filaments are set to nozzles
/// of two sizes: the reference then lays each filament's purge at its own
/// nozzle's width over a structure of the smallest (WipeTower.cpp
/// `tool_line_width`, `set_extruder`) and plans the tower as each nozzle's
/// own ladder of layers (`set_multi_diameter_print`, `merged_fill_active`,
/// WipeTower.cpp l.4510-5500), which is not done.
pub(crate) fn check_tower(settings: &Settings) -> Result<(), Failure> {
    let Some(sizes) = filament_sizes(settings)? else {
        return Ok(());
    };
    if sizes.iter().any(|size| (size - sizes[0]).abs() > 1e-6) {
        return Err(format!(
            "nozzles of two sizes: with filaments set to both the {} and the {} mm nozzle the \
             prime tower is laid at each nozzle's own width and planned as each nozzle's own \
             layers, which is not done yet; turn the prime tower off, or set every filament to \
             one nozzle; no G-code was written",
            size_text(sizes.iter().copied().fold(f64::INFINITY, f64::min)),
            size_text(sizes.iter().copied().fold(0.0, f64::max)),
        )
        .into());
    }
    Ok(())
}

/// What the prime tower reads, where every filament is set to one nozzle
/// of a printer whose nozzles differ in size: the settings with both
/// nozzles of that size, as each of the tower's widths is of the nozzle of
/// the filament it is for (WipeTower.cpp `set_extruder`,
/// `nozzle_diameter_for_filament`). None where the settings serve as they
/// are.
pub(crate) fn tower_view(settings: &Settings) -> Result<Option<Settings>, Failure> {
    let Some(sizes) = filament_sizes(settings)? else {
        return Ok(None);
    };
    let Some(size) = sizes.first().copied() else {
        return Ok(None);
    };
    if sizes.iter().any(|other| (other - size).abs() > 1e-6) {
        return Ok(None);
    }
    let mut view = settings.clone();
    view.insert("nozzle_diameter", Value::Many(vec![size_text(size); 2]));
    Ok(Some(view))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(path: &str) -> String {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer");
        std::fs::read_to_string(root.join(path)).unwrap()
    }

    /// The quoted names between `start` and the next `};` of a C++ source.
    fn quoted(text: &str, start: &str) -> Vec<String> {
        let body = &text[text.find(start).unwrap()..];
        let body = &body[..body.find("};").unwrap()];
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    fn profiles() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/resources/profiles")
    }

    #[test]
    fn per_nozzle_keys_are_the_references() {
        let config = reference("src/libslic3r/PrintConfig.cpp");
        let mut all = quoted(
            &config,
            "const std::vector<std::string>& process_flow_variant_options()",
        );
        let wide = quoted(&config, "static const std::set<std::string> print_wide");
        all.retain(|key| !wide.contains(key));
        assert_eq!(all, PER_NOZZLE_KEYS);
    }

    #[test]
    fn region_keys_are_the_references() {
        let preset = reference("src/libslic3r/Preset.cpp");
        let process = quoted(
            &preset,
            "static std::vector<std::string> s_Preset_print_options {",
        );
        let ours: Vec<&str> = region_keys().collect();
        let theirs: Vec<&str> = (crate::regions::REGION_KEYS.iter().copied())
            .filter(|key| process.iter().any(|own| own == key))
            .filter(|key| {
                !crate::regions::FEATURE_FILAMENTS.contains(key)
                    && !key.starts_with("nozzle_")
                    && !PER_NOZZLE_KEYS.contains(key)
                    && !WIDTH_KEYS.contains(key)
                    && !META.contains(key)
            })
            .collect();
        assert_eq!(ours, theirs);
        assert!(ours.contains(&"top_shell_layers"));
    }

    /// An H2D project's settings, as the reference's command line has them
    /// before its defaults: the 0.4 printer and process, two filaments, the
    /// right nozzle set to `right`.
    fn h2d(right: &str, map: [&str; 2]) -> Settings {
        let vendor = Vendor::open(&profiles(), "BBL").unwrap();
        let mut settings = vendor
            .resolve(Kind::Machine, "Bambu Lab H2D 0.4 nozzle")
            .unwrap();
        settings.overlay(
            &vendor
                .resolve(Kind::Process, "0.20mm Standard @BBL H2D")
                .unwrap(),
        );
        let many =
            |items: &[&str]| Value::Many(items.iter().map(|item| (*item).to_owned()).collect());
        settings.insert(
            "print_settings_id",
            Value::One("0.20mm Standard @BBL H2D".into()),
        );
        settings.insert("nozzle_diameter", many(&["0.4", right]));
        settings.insert("filament_diameter", many(&["1.75", "1.75"]));
        settings.insert("filament_map", many(&map));
        settings.insert("filament_map_mode", Value::One("Manual".into()));
        settings
    }

    #[test]
    fn the_second_size_prints_with_its_own_process() {
        let mut settings = h2d("0.6", ["1", "2"]);
        let notes = fill(&mut settings, Some(&profiles())).unwrap();
        assert!(
            notes[0].contains("'0.30mm Standard @BBL H2D 0.6 nozzle'"),
            "{notes:?}"
        );
        let texts = |key: &str| settings.texts(key).unwrap().to_vec();
        assert_eq!(texts("nozzle_outer_wall_line_width"), ["0.42", "0.62"]);
        assert_eq!(texts("nozzle_initial_layer_line_width"), ["0.5", "0.62"]);
        assert_eq!(
            texts("inner_wall_speed"),
            ["300", "600", "600", "300", "300", "300", "300"]
        );
        assert_eq!(settings.text("process_extruder_values"), Some("1"));
        let regions = texts("nozzle_region_values");
        assert!(regions[0].contains("top_shell_layers = 5"), "{regions:?}");
        assert!(regions[1].contains("top_shell_layers = 3"), "{regions:?}");
        let own = filament_overrides(&settings).unwrap().unwrap();
        assert!(own[0].is_empty(), "{:?}", own[0]);
        assert_eq!(own[1]["outer_wall_line_width"], "0.62");
        assert_eq!(own[1]["nozzle_diameter"], "0.6");
        assert_eq!(own[1]["top_shell_layers"], "3");
        // Read at its own slot (Right, Standard), not at its number's.
        assert_eq!(own[1]["inner_wall_speed"], "300");
    }

    /// A filament the map puts on the other nozzle takes that nozzle's
    /// process; the reference takes its number's nozzle when there are no
    /// more filaments than nozzles (work/tower/mix, h2_mix_21_nt).
    #[test]
    fn widths_follow_the_filament_map() {
        let mut settings = h2d("0.6", ["2", "1"]);
        fill(&mut settings, Some(&profiles())).unwrap();
        assert_eq!(
            settings.texts("nozzle_outer_wall_line_width").unwrap(),
            ["0.62", "0.42"]
        );
    }

    #[test]
    fn what_is_not_done_is_refused() {
        let mut auto = h2d("0.6", ["1", "2"]);
        auto.insert("filament_map_mode", Value::One("Auto For Flush".into()));
        let said = fill(&mut auto, Some(&profiles())).unwrap_err().to_string();
        assert!(
            said.contains("set the filaments' nozzles by hand"),
            "{said}"
        );
        let said = fill(&mut h2d("0.6", ["1", "2"]), None)
            .unwrap_err()
            .to_string();
        assert!(said.contains("--profiles"), "{said}");
        // No process of the printer is made for 0.5 mm nozzles.
        let said = fill(&mut h2d("0.5", ["1", "2"]), Some(&profiles()))
            .unwrap_err()
            .to_string();
        assert!(said.contains("Right (T2) has a 0.5 mm nozzle"), "{said}");
        // Nozzles of one size: nothing is filled.
        let mut same = h2d("0.4", ["1", "2"]);
        assert!(fill(&mut same, Some(&profiles())).unwrap().is_empty());
        assert!(same.get("nozzle_outer_wall_line_width").is_none());
    }

    #[test]
    fn layers_merged_into_slabs_are_refused() {
        let mut settings = h2d("0.6", ["1", "2"]);
        let many =
            |items: &[&str]| Value::Many(items.iter().map(|item| (*item).to_owned()).collect());
        // 0.6 / 0.4 is 1.4999... in doubles, so the ratio rounds to one
        // layer, as in the reference: at 0.2 mm the 0.6 nozzle prints every
        // layer, whatever its ceiling.
        settings.insert("max_layer_height", many(&["0.28", "0.42"]));
        check_merging(&settings, &[1, 2], &[1, 2], 0.2).unwrap();
        // 0.08 mm layers are thinner than the 0.6 nozzle's floor (0.12):
        // laid two at a time.
        let said = check_merging(&settings, &[1, 2], &[1, 2], 0.08)
            .unwrap_err()
            .to_string();
        assert!(said.contains("slabs of 0.16 mm"), "{said}");
        // All on one nozzle: nothing to merge.
        check_merging(&settings, &[2], &[2], 0.08).unwrap();
        assert_eq!(band_size(0.8, 0.4, 0.1, 0.16, 0.56), 2);
        assert_eq!(band_size(0.4, 0.4, 0.2, 0.08, 0.28), 1);
    }

    #[test]
    fn a_filament_reads_its_own_extruders_slot() {
        let layout = Layout {
            ids: vec![1, 1, 1, 2, 2, 2, 2],
            variants: [
                "Standard",
                "High Flow",
                "E3D High Flow",
                "Standard",
                "High Flow",
                "TPU High Flow",
                "E3D High Flow",
            ]
            .iter()
            .map(|kind| format!("Direct Drive {kind}"))
            .collect(),
            flow_support: Vec::new(),
        };
        assert_eq!(layout.extruder_count(), 2);
        assert_eq!(slot_of(&layout, 7, 0, Mode::Standard), 0);
        assert_eq!(slot_of(&layout, 7, 1, Mode::Standard), 3);
        assert_eq!(
            layout.index(7, 1, fill_mode("Direct Drive TPU High Flow")),
            3
        );
        assert_eq!(
            layout.index(7, 1, fill_mode("Direct Drive E3D High Flow")),
            4
        );
        assert_eq!(layout.index(1, 1, Mode::HighFlow), 0);
    }

    #[test]
    fn the_process_picked_is_of_the_plates_kind_and_height() {
        let candidates = [
            ("0.18mm Fine @BBL H2D 0.6 nozzle".to_owned(), 0.18),
            ("0.30mm Strength @BBL H2D 0.6 nozzle".to_owned(), 0.3),
            ("0.30mm Standard @BBL H2D 0.6 nozzle".to_owned(), 0.3),
            ("0.24mm Standard @BBL H2D 0.6 nozzle".to_owned(), 0.24),
        ];
        let picked = pick(&candidates, 0.2, "0.20mm Standard @BBL H2D").unwrap();
        assert_eq!(picked, "0.24mm Standard @BBL H2D 0.6 nozzle");
        let picked = pick(&candidates, 0.2, "0.20mm Strength @BBL H2D").unwrap();
        assert_eq!(picked, "0.30mm Strength @BBL H2D 0.6 nozzle");
        assert_eq!(quality_word("0.20mm Standard @BBL H2D"), "Standard");
    }

    /// The tower: laid as on a printer of the one size every filament is
    /// set to (work/tower/mix, h2_mix_four_2: 100 % of the reference's
    /// tower lines), refused with filaments on both sizes.
    #[test]
    fn the_tower_takes_the_one_size_the_filaments_print_from() {
        let both = h2d("0.6", ["1", "2"]);
        let said = check_tower(&both).unwrap_err().to_string();
        assert!(said.contains("turn the prime tower off"), "{said}");
        assert!(tower_view(&both).unwrap().is_none());
        let right = h2d("0.6", ["2", "2"]);
        check_tower(&right).unwrap();
        let view = tower_view(&right).unwrap().unwrap();
        assert_eq!(view.texts("nozzle_diameter").unwrap(), ["0.6", "0.6"]);
        // Nozzles of one size: nothing to do.
        assert!(tower_view(&h2d("0.4", ["1", "2"])).unwrap().is_none());
    }
}
