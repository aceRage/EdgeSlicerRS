//! One value per filament and one per extruder, from presets that keep a
//! value per flow variant or per nozzle variant.
//!
//! How the reference turns a printer, a process and N filament presets into
//! the flat settings a print uses (PresetBundle.cpp `full_fff_config`,
//! Snapmaker_Orca.cpp's named-preset path, PrintConfig.cpp `get_config_idx`
//! and `resolve_printer_extruder_variants`, PresetFlowVariant.cpp,
//! BambuFlowSupport.cpp):
//!
//! - **Filament presets.** A filament preset holds one filament. Most of its
//!   lists have one entry, but the 23 flow-variant settings
//!   ([`FILAMENT_FLOW_VARIANT_KEYS`]: temperatures, flow ratio, volumetric
//!   speed, fan speeds, the retraction overrides, ramming) may hold one entry
//!   per flow variant, in the order `filament_flow_support` names them
//!   (`["standard", "high_flow"]` on most of the U1's Snapmaker presets). A
//!   Bambu preset says the same with `filament_extruder_variant`
//!   (`["Direct Drive Standard", "Direct Drive High Flow", ...]`); when its
//!   `filament_flow_support` is left as it was and it has two volumetric
//!   speeds, the reference takes it as `["standard", "high_flow"]`
//!   (`BambuFlowSupport::derive`, at preset load). Other lists may carry
//!   further entries too (a Bambu preset's `filament_extruder_variant`,
//!   `filament_ramming_travel_time`); nothing reads past the first.
//! - **Which entry.** Each filament has a flow type, `filament_volume_type`
//!   (one per filament, `standard` unless the person chose high flow;
//!   standard past the list's end). A flow-variant setting is read at the
//!   position of that type in the filament's own `filament_flow_support`,
//!   the first when the type is not there; past a list's end, and for a
//!   `nil` there over a value in the first, the first entry. Every other
//!   list is read at its first entry. A setting that is not a list comes from
//!   filament 1.
//! - **The flat list.** The reference's application packs each filament's
//!   flow variants side by side (`filament_flow_step_size` long each) and
//!   picks the entry when it reads; its command line, given presets by name,
//!   writes one entry per filament, the first (Snapmaker_Orca.cpp, the
//!   `set_at(opt_vec_src, filament_index-1, 0)` of its filament loop), so
//!   it prints a filament set to high flow at its standard values and
//!   leaves a project's longer lists' tails in place. That is not followed:
//!   a filament at high flow reads its high-flow entry here, as in the
//!   application. Both read the same value for a standard filament. Here every list is made one entry per filament, in
//!   filament order, holding the entry that filament reads, so everything
//!   downstream (and what is set for one filament alone) indexes it by
//!   filament. `filament_flow_support` becomes the variant each filament
//!   reads, and `filament_flow_step_size` goes, which together read back the
//!   same under the reference's own rule.
//! - **Printer presets.** Per extruder lists have one entry per extruder,
//!   except on a printer laid out per (extruder, nozzle variant)
//!   (`printer_extruder_id` / `printer_extruder_variant`, Bambu's H2D: seven
//!   slots for two extruders). Its 20 retraction settings
//!   ([`PRINTER_EXTRUDER_VARIANT_KEYS`]) hold a value per slot, and each
//!   extruder reads the slot of its own variant: `extruder_type` and
//!   `nozzle_volume_type` ("Direct Drive Standard"), its first slot when it
//!   has no such variant. Other slot lists (`nozzle_volume`,
//!   `hotend_heating_rate`) are left as they are.
//! - **Process presets** keep their own per-variant lists
//!   (`process_flow_support`); they are not resolved here.

use std::collections::BTreeMap;

use crate::{Settings, Value};

/// PrintConfig.cpp `filament_flow_variant_options`.
pub const FILAMENT_FLOW_VARIANT_KEYS: [&str; 23] = [
    "filament_flow_ratio",
    "enable_pressure_advance",
    "pressure_advance",
    "nozzle_temperature_initial_layer",
    "nozzle_temperature",
    "filament_max_volumetric_speed",
    "fan_min_speed",
    "fan_max_speed",
    "additional_cooling_fan_speed",
    "filament_retraction_length",
    "filament_retraction_speed",
    "filament_deretraction_speed",
    "filament_z_hop_types",
    "filament_wipe_distance",
    "filament_retract_length_toolchange",
    "filament_multitool_ramming",
    "filament_multitool_ramming_volume",
    "filament_multitool_ramming_flow",
    "filament_minimal_purge_on_wipe_tower",
    "filament_flush_volumetric_speed",
    "filament_flush_temp",
    "filament_flush_temp_fast",
    "filament_cooling_before_tower",
];

/// PrintConfig.cpp `is_printer_extruder_variant_option`.
pub const PRINTER_EXTRUDER_VARIANT_KEYS: [&str; 20] = [
    "retraction_length",
    "z_hop",
    "travel_slope",
    "retract_lift_above",
    "retract_lift_below",
    "retract_lift_enforce",
    "z_hop_types",
    "retraction_speed",
    "deretraction_speed",
    "retraction_minimum_travel",
    "retract_when_changing_layer",
    "wipe",
    "wipe_distance",
    "retract_before_wipe",
    "retract_after_wipe",
    "retract_length_toolchange",
    "retract_restart_extra",
    "retract_restart_extra_toolchange",
    "long_retractions_when_cut",
    "retraction_distances_when_cut",
];

/// Lists that name what a preset suits rather than describe a filament;
/// the reference leaves them out of the per-filament lists.
const NOT_PER_FILAMENT: [&str; 2] = ["compatible_printers", "compatible_prints"];

/// The flow a filament prints at (`filament_volume_type`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Flow {
    #[default]
    Standard,
    HighFlow,
}

impl Flow {
    /// The name `filament_volume_type` and `filament_flow_support` use.
    pub fn name(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::HighFlow => "high_flow",
        }
    }

    /// Each filament's flow from a `filament_volume_type` list.
    pub fn list(texts: &[String]) -> Result<Vec<Self>, String> {
        texts
            .iter()
            .map(|text| match text.trim() {
                "standard" => Ok(Self::Standard),
                "high_flow" => Ok(Self::HighFlow),
                other => Err(format!(
                    "filament_volume_type holds \"{other}\", which is neither standard nor high_flow"
                )),
            })
            .collect()
    }
}

/// A preset's settings by name, read without noting anything as asked for
/// ([`Settings::noted_by`]): which entry is taken is not a read of the
/// setting, and the print notes what it reads of the result.
fn table(settings: &Settings) -> BTreeMap<&str, &Value> {
    settings.iter().collect()
}

fn entries<'a>(table: &BTreeMap<&str, &'a Value>, key: &str) -> Option<&'a [String]> {
    Some(match table.get(key)? {
        Value::One(text) => std::slice::from_ref(text),
        Value::Many(texts) => texts,
    })
}

/// The flow variants a filament preset has values for, in the order its
/// flow-variant lists hold them (PresetFlowVariant.cpp, and
/// BambuFlowSupport.cpp `derive` for a Bambu preset that says it by
/// `filament_extruder_variant`).
fn flow_support(preset: &BTreeMap<&str, &Value>) -> Vec<String> {
    let own = entries(preset, "filament_flow_support")
        .filter(|support| !support.is_empty())
        .map(<[String]>::to_vec);
    let unauthored = own.as_deref().is_none_or(|support| support == ["standard"]);
    if unauthored {
        let names = entries(preset, "filament_extruder_variant").unwrap_or(&[]);
        let speeds = entries(preset, "filament_max_volumetric_speed").map_or(0, <[String]>::len);
        if standard_then_high_flow(names) && speeds >= 2 {
            return vec!["standard".into(), "high_flow".into()];
        }
    }
    own.unwrap_or_else(|| vec!["standard".into()])
}

/// BambuFlowSupport.cpp `variant_names_standard_high_flow`: "<type>
/// Standard" then "<type> High Flow", where a TPU or E3D high-flow nozzle
/// is a variant of its own.
fn standard_then_high_flow(names: &[String]) -> bool {
    let [standard, high, ..] = names else {
        return false;
    };
    let (Some(kind), Some(high_kind)) = (
        standard.strip_suffix(" Standard"),
        high.strip_suffix(" High Flow"),
    ) else {
        return false;
    };
    let qualified = |kind: &str| {
        kind.ends_with(" TPU") || kind.ends_with(" E3D") || kind == "TPU" || kind == "E3D"
    };
    !kind.is_empty() && kind == high_kind && !qualified(high_kind)
}

/// The entry a filament reads of a list holding one per flow variant, the
/// variant being at `index` (PresetFlowVariant.cpp
/// `compose_filament_flow_variant_segment`): the first past the list's end,
/// and the first over a `nil` when the first is not `nil`.
fn variant_entry(list: &[String], index: usize) -> Option<&String> {
    let first = list.first()?;
    match list.get(index) {
        Some(value) if value == "nil" && first != "nil" => Some(first),
        Some(value) => Some(value),
        None => Some(first),
    }
}

/// The settings `count` filaments print with, from a preset for each
/// (`presets[n]` for filament n + 1, the last for any past the end) and the
/// flow each prints at (`flows[n]`, standard past the end): every list one
/// entry per filament, as the module says. Also what the person should be
/// told: a filament set to high flow whose preset has no high-flow values.
pub fn for_filaments(
    presets: &[(&str, &Settings)],
    count: usize,
    flows: &[Flow],
) -> (Settings, Vec<String>) {
    let Some(&(_, first)) = presets.first() else {
        return (Settings::default(), Vec::new());
    };
    let tables: Vec<BTreeMap<&str, &Value>> =
        presets.iter().map(|(_, preset)| table(preset)).collect();
    let supports: Vec<Vec<String>> = tables.iter().map(flow_support).collect();
    let preset_of = |filament: usize| filament.min(presets.len() - 1);
    let flow_of = |filament: usize| flows.get(filament).copied().unwrap_or_default();
    let mut notes = Vec::new();
    // Where in its preset's lists each filament's flow variant is: the
    // first when the preset has none of that flow (PrintConfig.cpp
    // `flow_variant_index`), which the reference reports
    // (BambuFlowSupport.cpp `filaments_without_high_flow_column`).
    let variant: Vec<usize> = (0..count)
        .map(|filament| {
            let flow = flow_of(filament);
            let support = &supports[preset_of(filament)];
            support
                .iter()
                .position(|name| name == flow.name())
                .unwrap_or_else(|| {
                    notes.push(format!(
                        "filament {} is set to print at {}, which its preset \"{}\" has no values for; its {} values are used",
                        filament + 1,
                        flow.name(),
                        presets[preset_of(filament)].0,
                        support.first().map_or("standard", String::as_str),
                    ));
                    0
                })
        })
        .collect();

    let mut merged = first.clone();
    // Every list any of the presets has, so a filament whose preset has a
    // setting the first lacks keeps its own value.
    let mut lists: Vec<&str> = tables
        .iter()
        .flat_map(|table| {
            table
                .iter()
                .filter(|(_, value)| matches!(value, Value::Many(_)))
                .map(|(key, _)| *key)
        })
        .filter(|key| !NOT_PER_FILAMENT.contains(key))
        .collect();
    lists.sort_unstable();
    lists.dedup();
    for key in lists {
        // A setting that is one value in the first preset stays its value.
        if matches!(tables[0].get(key), Some(Value::One(_))) {
            continue;
        }
        let flow_variant = FILAMENT_FLOW_VARIANT_KEYS.contains(&key);
        let values = (0..count)
            .map(|filament| {
                let preset = preset_of(filament);
                if key == "filament_flow_support" {
                    return supports[preset][variant[filament]].clone();
                }
                let index = if flow_variant { variant[filament] } else { 0 };
                match entries(&tables[preset], key).and_then(|list| variant_entry(list, index)) {
                    Some(value) => value.clone(),
                    // A preset without the setting has the reference's
                    // default for it, as every preset the reference loads
                    // is completed with them (Preset.cpp `normalize`).
                    None => crate::reference_defaults()
                        .texts(key)
                        .and_then(<[String]>::first)
                        .or_else(|| tables.iter().find_map(|table| entries(table, key)?.first()))
                        .cloned()
                        .unwrap_or_default(),
                }
            })
            .collect();
        merged.insert(key, Value::Many(values));
    }
    // The variant each filament reads, also for presets that name none.
    let read = (0..count).map(|filament| supports[preset_of(filament)][variant[filament]].clone());
    merged.insert("filament_flow_support", Value::Many(read.collect()));
    merged.remove("filament_flow_step_size");
    (merged, notes)
}

/// Makes a project's filament lists packed per flow variant one entry per
/// filament: a project the reference's application saved holds each
/// filament's variants side by side, `filament_flow_step_size[n]` of them
/// for filament n + 1 (PresetBundle.cpp `full_fff_config`), and is read by
/// PrintConfig.cpp `get_config_idx`. Settings whose step sizes are all 1
/// are one entry per filament already and are left exactly as they are.
pub fn unpack_flow_variants(settings: &mut Settings, flows: &[Flow]) {
    let steps: Vec<usize> = match settings
        .iter()
        .find(|(key, _)| *key == "filament_flow_step_size")
    {
        Some((_, Value::Many(steps))) => steps
            .iter()
            .map(|step| step.trim().parse::<usize>().unwrap_or(1).max(1))
            .collect(),
        Some((_, Value::One(step))) => vec![step.trim().parse::<usize>().unwrap_or(1).max(1)],
        None => return,
    };
    if steps.iter().all(|step| *step == 1) {
        return;
    }
    let count = steps.len();
    let starts: Vec<usize> = steps
        .iter()
        .scan(0, |start, step| {
            let this = *start;
            *start += step;
            Some(this)
        })
        .collect();
    let support: Vec<String> = settings
        .iter()
        .find(|(key, _)| *key == "filament_flow_support")
        .map(|(_, value)| match value {
            Value::One(text) => vec![text.clone()],
            Value::Many(texts) => texts.clone(),
        })
        .unwrap_or_default();
    // PrintConfig.cpp `get_config_idx`, Filament: the filament's segment,
    // at its flow's place there, the segment's start when it is not there.
    let index: Vec<usize> = (0..count)
        .map(|filament| {
            let flow = flows.get(filament).copied().unwrap_or_default();
            (0..steps[filament])
                .map(|at| starts[filament] + at)
                .find(|at| support.get(*at).is_some_and(|name| name == flow.name()))
                .unwrap_or(starts[filament])
        })
        .collect();
    let keys = FILAMENT_FLOW_VARIANT_KEYS
        .iter()
        .copied()
        .chain(["filament_flow_support"]);
    for key in keys {
        let Some((_, Value::Many(packed))) = settings.iter().find(|(name, _)| *name == key) else {
            continue;
        };
        if packed.is_empty() {
            continue;
        }
        let packed = packed.clone();
        let values = index
            .iter()
            .map(|at| packed.get(*at).unwrap_or(&packed[packed.len() - 1]).clone())
            .collect();
        settings.insert(key, Value::Many(values));
    }
    settings.remove("filament_flow_step_size");
}

/// Makes a printer's lists laid out per (extruder, nozzle variant) one
/// entry per extruder, each extruder's own variant's (PrintConfig.cpp
/// `resolve_printer_extruder_variants`, `printer_extruder_variant_slot`).
/// Returns what the person should be told: an extruder whose variant the
/// printer has no values for.
///
/// The reference differs on Bambu's grouping printers (H2D, H2C, X2D:
/// `support_different_extruders`), where it takes slot `e` for extruder
/// `e` whatever the slots are (PrintConfig.cpp
/// `printer_extruder_variant_sources`), so the H2D's right extruder reads
/// the left one's High Flow slot. Here every printer's extruder reads its
/// own slot. The shipped H2D presets hold the same retraction values in
/// every slot, so nothing printed differs by it.
pub fn for_extruders(settings: &mut Settings) -> Vec<String> {
    let mut notes = Vec::new();
    let table = table(settings);
    let texts = |key: &str| entries(&table, key).unwrap_or(&[]);
    let single = texts("single_extruder_multi_material");
    if single.first().is_none_or(|single| single.trim() != "0") {
        return notes;
    }
    let (ids, variants) = (
        texts("printer_extruder_id"),
        texts("printer_extruder_variant"),
    );
    if ids.is_empty() || ids.len() != variants.len() {
        return notes;
    }
    // printer_extruder_variant_slots: the slots of extruder 1, then 2...
    let mut slots: Vec<Vec<usize>> = Vec::new();
    for (slot, id) in ids.iter().enumerate() {
        let Ok(id) = id.trim().parse::<usize>() else {
            return notes;
        };
        if id == slots.len() + 1 {
            slots.push(Vec::new());
        } else if id != slots.len() {
            return notes;
        }
        slots.last_mut().expect("an extruder is open").push(slot);
    }
    if slots.len() == ids.len() {
        return notes;
    }
    if texts("nozzle_diameter").len() != slots.len() {
        notes.push(format!(
            "the printer's retraction settings are given for {} extruders' nozzle variants but it has {} nozzles; they are read as they are",
            slots.len(),
            texts("nozzle_diameter").len()
        ));
        return notes;
    }
    let sources: Vec<usize> = slots
        .iter()
        .enumerate()
        .map(|(extruder, own)| {
            let kind = match texts("extruder_type").get(extruder).map(|kind| kind.trim()) {
                Some("Bowden") => "Bowden",
                _ => "Direct Drive",
            };
            let volume = match texts("nozzle_volume_type")
                .get(extruder)
                .map(|volume| volume.trim())
            {
                // Bambu: a hybrid nozzle is not a variant of its own.
                None | Some("Hybrid") | Some("") => "Standard",
                Some(volume) => volume,
            };
            let wanted = format!("{kind} {volume}");
            own.iter()
                .copied()
                .find(|slot| variants[*slot] == wanted)
                .unwrap_or_else(|| {
                    notes.push(format!(
                        "extruder {} is a \"{wanted}\", which the printer has no retraction values for; those of its \"{}\" are used",
                        extruder + 1,
                        variants[own[0]]
                    ));
                    own[0]
                })
        })
        .collect();
    let resolved: Vec<(&str, Vec<String>)> = PRINTER_EXTRUDER_VARIANT_KEYS
        .iter()
        .filter_map(|key| match table.get(key) {
            Some(Value::Many(list)) if list.len() == ids.len() => Some((
                *key,
                sources.iter().map(|slot| list[*slot].clone()).collect(),
            )),
            _ => None,
        })
        .collect();
    for (key, values) in resolved {
        settings.insert(key, Value::Many(values));
    }
    notes
}

#[cfg(test)]
mod tests;
