//! The nozzles of a nozzle rack (the Bambu Lab H2C's right extruder: six
//! hotends it swaps between), as the project, the printer or the machine
//! sets them, and the sizes filaments are pinned to (ToolSlots.cpp, the
//! reference's "rack nozzle diameters", P4 and P9).
//!
//! - A rack is a text of nozzles, "0.4,0.4,0.6:high_flow", per extruder:
//!   the project's (`extruder_rack_nozzle_override`) over the printer's
//!   (`extruder_rack_nozzle_diameter`) over the machine's own
//!   ([`default_text`]: `extruder_max_nozzle_count` nozzles of the
//!   extruder's size and kind).
//! - A filament may be pinned to a size of its extruder's rack
//!   (`filament_rack_nozzle_diameter`) or to one of its slots
//!   (`filament_rack_nozzle_slot`, from 1). It prints with that size (its
//!   walls and fill are laid for it, `crate::nozzle_process`) and on a
//!   nozzle of it: the grouping is moved onto such nozzles ([`pin`]) and
//!   checked ([`check`]).
//!
//! What is refused here, each with what to do: a rack text on an extruder
//! that holds fewer nozzles (the reference takes the text as it is), a
//! rack of mixed nozzle kinds (Hybrid, ":high_flow" entries: not done
//! yet), a pin on an automatic filament map (see [`check_pins`]), and a
//! grouping that puts one extruder's filaments on rack nozzles of two
//! sizes ([`check_one_size`]), which the reference only warns of.

use edge_config::Settings;
use edge_group::{GroupResult, NozzleInfo, VolumeType};

use crate::Failure;

/// Two diameters are one size (ToolSlots.cpp `same_nozzle_diameter`).
pub(crate) fn same(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.001
}

/// One nozzle of a rack text: its diameter and, where the text says
/// ":<kind>", its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RackNozzle {
    pub diameter: f64,
    pub kind: VolumeType,
    pub kind_given: bool,
}

/// ToolSlots.cpp `parse_nozzle_volume`.
fn kind_word(word: &str) -> Option<VolumeType> {
    let word = word.trim().to_lowercase().replace(' ', "_");
    Some(match word.as_str() {
        "standard" | "std" => VolumeType::STANDARD,
        "high_flow" | "hf" => VolumeType::HIGH_FLOW,
        "tpu_high_flow" => VolumeType::TPU_HIGH_FLOW,
        "tpu285" => VolumeType::TPU_285,
        "e3d_high_flow" => VolumeType::E3D_HIGH_FLOW,
        _ => return None,
    })
}

/// The reference's `parse_positive_number`: the whole word a number above 0.
fn positive(word: &str) -> Option<f64> {
    word.trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > 0.0)
}

/// ToolSlots.cpp `parse_rack_nozzles`: the nozzles of a rack text, entries
/// that are not a positive number skipped; a kind it cannot read is
/// Standard and not given, as there.
pub(crate) fn parse(text: &str) -> Vec<RackNozzle> {
    text.split(',')
        .filter_map(|entry| {
            let entry = entry.trim();
            let (size, kind) = match entry.split_once(':') {
                Some((size, kind)) => (size, Some(kind)),
                None => (entry, None),
            };
            let diameter = positive(size)?;
            let kind = kind.and_then(kind_word);
            Some(RackNozzle {
                diameter,
                kind: kind.unwrap_or(VolumeType::STANDARD),
                kind_given: kind.is_some(),
            })
        })
        .collect()
}

fn numbers(settings: &Settings, key: &str) -> Vec<f64> {
    (settings.floats(key).ok().flatten().unwrap_or_default())
        .into_iter()
        .map(|value| value.unwrap_or(0.0))
        .collect()
}

/// How many nozzles each extruder holds (`extruder_max_nozzle_count`, at
/// least one).
pub(crate) fn counts(settings: &Settings) -> Vec<i32> {
    numbers(settings, "extruder_max_nozzle_count")
        .into_iter()
        .map(|count| (count as i32).max(1))
        .collect()
}

/// An extruder's kind of nozzle (`nozzle_volume_type`, Standard where the
/// list has no entry).
fn extruder_kind(settings: &Settings, extruder: usize) -> VolumeType {
    (settings.texts("nozzle_volume_type"))
        .and_then(|kinds| kinds.get(extruder))
        .and_then(|name| VolumeType::from_name(name.trim()))
        .unwrap_or(VolumeType::STANDARD)
}

/// A diameter as the reference writes it in a rack text and in messages
/// (`format_nozzle_diameter_word`, `format_diameter_to_str`): "0.4", "0.25".
pub(crate) fn size_text(diameter: f64) -> String {
    let text = format!("{diameter:.2}");
    let text = text.trim_end_matches('0');
    text.trim_end_matches('.').to_owned()
}

/// ToolSlots.cpp `default_rack_nozzle_text`: the rack an extruder has when
/// neither the project nor the printer names one, `extruder_max_nozzle_count`
/// nozzles of its size; none for an extruder without a rack or a Hybrid
/// one. The per-slot kinds of a `default_nozzle_volume_type` as long as all
/// slots (the H3C's) are refused before this is read
/// (`crate::grouping::machine`).
pub(crate) fn default_text(settings: &Settings, extruder: usize) -> String {
    let counts = counts(settings);
    let Some(&count) = counts.get(extruder).filter(|count| **count >= 2) else {
        return String::new();
    };
    if extruder_kind(settings, extruder) == VolumeType::HYBRID {
        return String::new();
    }
    let Some(&size) = numbers(settings, "nozzle_diameter")
        .get(extruder)
        .filter(|size| **size > 0.0)
    else {
        return String::new();
    };
    vec![size_text(size); count as usize].join(",")
}

/// An extruder's rack text (ToolSlots.cpp `rack_text_of`), and the key it
/// came from (none for the machine's own).
pub(crate) fn text_of(settings: &Settings, extruder: usize) -> (String, Option<&'static str>) {
    for key in [
        "extruder_rack_nozzle_override",
        "extruder_rack_nozzle_diameter",
    ] {
        if let Some(text) = (settings.texts(key))
            .and_then(|texts| texts.get(extruder))
            .filter(|text| !text.trim().is_empty())
        {
            return (text.clone(), Some(key));
        }
    }
    (default_text(settings, extruder), None)
}

/// Each extruder's rack (ToolSlots.cpp `extruder_rack_nozzles`): empty for
/// one without; a nozzle whose kind the text does not give is of the
/// extruder's kind (Standard for a Hybrid one).
pub(crate) fn racks(settings: &Settings) -> Vec<Vec<RackNozzle>> {
    let extruders = numbers(settings, "nozzle_diameter").len();
    (0..extruders)
        .map(|extruder| {
            let mut kind = extruder_kind(settings, extruder);
            if kind == VolumeType::HYBRID {
                kind = VolumeType::STANDARD;
            }
            let mut rack = parse(&text_of(settings, extruder).0);
            for nozzle in &mut rack {
                if !nozzle.kind_given {
                    nozzle.kind = kind;
                }
            }
            rack
        })
        .collect()
}

/// Whether some rack holds nozzles of more than one size
/// (`has_mixed_rack_diameters`).
pub(crate) fn mixed_sizes(racks: &[Vec<RackNozzle>]) -> bool {
    racks
        .iter()
        .any(|rack| rack.iter().any(|n| !same(n.diameter, rack[0].diameter)))
}

/// Each filament's extruder, from 0, by `filament_map` (`crate::nozzle_process::extruders_of`;
/// the reference's `physical_extruder_for_filament` takes the filament's
/// own number where there are no more filaments than nozzles, a bug that
/// is not copied, see there).
fn extruder_of(settings: &Settings, filament: usize) -> usize {
    let extruders = numbers(settings, "nozzle_diameter").len().max(1);
    crate::nozzle_process::extruders_of(settings, extruders)
        .get(filament)
        .copied()
        .unwrap_or(0)
}

/// The slot `filament` (from 0) is pinned to, from 1; 0 for none
/// (`filament_rack_slot_pin`).
pub(crate) fn slot_pin(settings: &Settings, filament: usize) -> usize {
    (settings.texts("filament_rack_nozzle_slot"))
        .and_then(|pins| pins.get(filament))
        .and_then(|pin| pin.trim().parse::<i64>().ok())
        .map_or(0, |pin| pin.max(0) as usize)
}

/// Whether some filament is pinned to a slot (`has_filament_rack_slot_pins`).
pub(crate) fn has_slot_pins(settings: &Settings) -> bool {
    let count = settings
        .texts("filament_rack_nozzle_slot")
        .map_or(0, <[String]>::len);
    (0..count).any(|filament| slot_pin(settings, filament) > 0)
}

/// The rack size `filament` (from 0) is pinned to, 0 for none
/// (`filament_rack_pin`): its slot's size, else `filament_rack_nozzle_diameter`.
pub(crate) fn size_pin(settings: &Settings, racks: &[Vec<RackNozzle>], filament: usize) -> f64 {
    let slot = slot_pin(settings, filament);
    if slot > 0
        && let Some(nozzle) = racks
            .get(extruder_of(settings, filament))
            .and_then(|rack| rack.get(slot - 1))
    {
        return nozzle.diameter;
    }
    numbers(settings, "filament_rack_nozzle_diameter")
        .get(filament)
        .copied()
        .filter(|pin| *pin > 0.0)
        .unwrap_or(0.0)
}

/// Whether some filament is pinned to a size or a slot at all.
pub(crate) fn any_pins(settings: &Settings) -> bool {
    has_slot_pins(settings)
        || numbers(settings, "filament_rack_nozzle_diameter")
            .iter()
            .any(|pin| *pin > 0.0)
}

/// The size of nozzle `filament` (from 0) prints with
/// (`nozzle_diameter_for_filament`): its extruder's, or the rack size it is
/// pinned to where its extruder's rack has one.
pub(crate) fn filament_size(
    settings: &Settings,
    racks: &[Vec<RackNozzle>],
    filament: usize,
) -> f64 {
    let nozzles = numbers(settings, "nozzle_diameter");
    let Some(last) = nozzles.len().checked_sub(1) else {
        return 0.0;
    };
    let extruder = extruder_of(settings, filament).min(last);
    let own = nozzles[extruder];
    let rack = racks.get(extruder).map_or(&[][..], Vec::as_slice);
    let pin = size_pin(settings, racks, filament);
    if rack.is_empty() || pin <= 0.0 {
        return own;
    }
    rack.iter()
        .find(|nozzle| same(nozzle.diameter, pin))
        .map_or(own, |nozzle| nozzle.diameter)
}

/// "Right slot 3" (`rack_slot_name` on Bambu's two-extruder printers).
fn slot_name(extruders: usize, extruder: usize, slot: usize) -> String {
    let side = if extruders == 2 {
        if extruder == 0 {
            "Left".to_owned()
        } else {
            "Right".to_owned()
        }
    } else {
        format!("T{}", extruder + 1)
    };
    format!("{side} slot {slot}")
}

/// ToolSlots.cpp `validate_rack_nozzle_diameters`, which Print::validate
/// refuses a slice with (Print.cpp l.3442), and what this engine refuses
/// besides: a rack text of more nozzles than its extruder holds, and
/// nozzles of a kind other than the extruder's (stage 4). `used` are the
/// filaments the plate prints, from 0.
pub(crate) fn validate(settings: &Settings, used: &[u32]) -> Result<(), Failure> {
    let racks = racks(settings);
    let nozzles = numbers(settings, "nozzle_diameter");
    let counts = counts(settings);
    for (extruder, rack) in racks.iter().enumerate() {
        if rack.is_empty() {
            continue;
        }
        let (_, key) = text_of(settings, extruder);
        let key = key.unwrap_or("extruder_max_nozzle_count");
        let holds = counts.get(extruder).copied().unwrap_or(1) as usize;
        if rack.len() > holds {
            return Err(format!(
                "{key}: extruder {}'s rack lists {} nozzles, but it holds {holds} \
                 (extruder_max_nozzle_count); list at most {holds}; no G-code was written",
                extruder + 1,
                rack.len()
            )
            .into());
        }
        let own_kind = {
            let kind = extruder_kind(settings, extruder);
            if kind == VolumeType::HYBRID {
                VolumeType::STANDARD
            } else {
                kind
            }
        };
        if rack.iter().any(|nozzle| nozzle.kind != own_kind) {
            return Err(format!(
                "{key}: extruder {}'s rack holds nozzles of another kind than its own: a rack \
                 of mixed nozzle kinds is not done yet; give every rack nozzle the extruder's \
                 kind; no G-code was written",
                extruder + 1
            )
            .into());
        }
        let own = nozzles.get(extruder).copied().unwrap_or(0.0);
        if !rack.iter().any(|nozzle| same(nozzle.diameter, own)) {
            return Err(format!(
                "Extruder {}: its nozzle diameter {} mm is not one of its rack nozzle \
                 diameters. ({key}); no G-code was written",
                extruder + 1,
                size_text(own)
            )
            .into());
        }
    }
    for filament in used {
        let filament = *filament as usize;
        let pin = size_pin(settings, &racks, filament);
        if pin <= 0.0 {
            continue;
        }
        let extruder = extruder_of(settings, filament);
        let Some(rack) = racks.get(extruder).filter(|rack| !rack.is_empty()) else {
            continue;
        };
        if !rack.iter().any(|nozzle| same(nozzle.diameter, pin)) {
            return Err(format!(
                "Filament {} asks for a {} mm rack nozzle, but extruder {}'s rack has none. \
                 (filament_rack_nozzle_diameter); no G-code was written",
                filament + 1,
                size_text(pin),
                extruder + 1
            )
            .into());
        }
    }
    for filament in used {
        let filament = *filament as usize;
        let slot = slot_pin(settings, filament);
        if slot == 0 {
            continue;
        }
        let extruder = extruder_of(settings, filament);
        let holds = racks.get(extruder).map_or(0, Vec::len);
        if slot > holds {
            return Err(format!(
                "Filament {} is set to print on {}, but that rack has {holds} nozzles. \
                 (filament_rack_nozzle_slot); no G-code was written",
                filament + 1,
                slot_name(racks.len(), extruder, slot)
            )
            .into());
        }
    }
    Ok(())
}

/// A filament pinned to a rack size or slot on an automatic filament map
/// is refused. The reference reads the pin through the project's
/// `filament_map` as saved, not through the map it is grouping: measured
/// on h2c3_mix2_auto_pn (filament 1 pinned to 0.6, saved map 1,2,1), it
/// grouped filament 1 onto the right extruder and printed it from a 0.4
/// nozzle, its pin dropped without a word (ToolSlots.cpp l.497
/// `filament_rack_pin`, `physical_extruder_of` on the config's map). Which
/// extruder a pinned filament may go to is not something the grouping
/// knows; that is left out.
pub(crate) fn check_pins(settings: &Settings, manual: bool) -> Result<(), Failure> {
    if !manual && any_pins(settings) {
        return Err(
            "a filament is pinned to a rack nozzle size or slot (filament_rack_nozzle_diameter, \
             filament_rack_nozzle_slot) on an automatic filament map: which extruder it prints \
             from is only settled by the grouping, which does not know the pin; set the \
             filaments' nozzles by hand (Manual); no G-code was written"
                .into(),
        );
    }
    Ok(())
}

/// The place in `nozzles` of `extruder`'s rack slot `slot` (from 1)
/// (`rack_slot_nozzle_index`). The list is by physical slot
/// ([`crate::grouping`] numbers it so), so it is that extruder's `slot`-th
/// nozzle.
fn slot_index(nozzles: &[NozzleInfo], extruder: i32, slot: usize) -> Option<usize> {
    (0..nozzles.len())
        .filter(|n| nozzles[*n].extruder_id == extruder)
        .nth(slot.checked_sub(1)?)
}

/// ToolSlots.cpp `pin_rack_nozzle_diameters`: a used filament pinned to a
/// slot goes to that slot's nozzle of the extruder the grouping put it on;
/// on a rack of mixed sizes, one on a nozzle of another size than it
/// prints with ([`filament_size`]) goes to the first nozzle of its
/// extruder of that size (of the same kind where there is one). Returns how
/// many moved.
pub(crate) fn pin(settings: &Settings, result: &mut GroupResult) -> usize {
    let racks = racks(settings);
    let slot_pins = has_slot_pins(settings);
    let mixed = mixed_sizes(&racks);
    if !mixed && !slot_pins {
        return 0;
    }
    let mut moved = 0;
    for filament in result.used.clone() {
        let f = filament as usize;
        let Some(&current) = result.map.get(f) else {
            continue;
        };
        let Some(now) = usize::try_from(current)
            .ok()
            .and_then(|n| result.nozzles.get(n))
            .cloned()
        else {
            continue;
        };
        let slot = if slot_pins { slot_pin(settings, f) } else { 0 };
        if slot > 0
            && let Some(target) = slot_index(&result.nozzles, now.extruder_id.max(0), slot)
        {
            if target as i32 != current {
                result.map[f] = target as i32;
                moved += 1;
            }
            continue;
        }
        if !mixed {
            continue;
        }
        let want = filament_size(settings, &racks, f);
        if positive(&now.diameter).is_some_and(|have| same(have, want)) {
            continue;
        }
        let of_size: Vec<usize> = (0..result.nozzles.len())
            .filter(|n| {
                let nozzle = &result.nozzles[*n];
                nozzle.extruder_id == now.extruder_id
                    && positive(&nozzle.diameter).is_some_and(|d| same(d, want))
            })
            .collect();
        let target = (of_size.iter())
            .find(|n| result.nozzles[**n].volume_type == now.volume_type)
            .or(of_size.first());
        if let Some(target) = target {
            result.map[f] = *target as i32;
            moved += 1;
        }
    }
    moved
}

/// ToolSlots.cpp `check_rack_nozzle_diameters`, which the reference throws
/// (ToolOrdering.cpp l.2155): every used filament prints on a nozzle of the
/// size its walls were laid for.
pub(crate) fn check(settings: &Settings, result: &GroupResult) -> Result<(), Failure> {
    let racks = racks(settings);
    if !mixed_sizes(&racks) {
        return Ok(());
    }
    for filament in &result.used {
        let Some(have) = result
            .nozzle_for(i64::from(*filament))
            .and_then(|nozzle| positive(&nozzle.diameter))
        else {
            continue;
        };
        let want = filament_size(settings, &racks, *filament as usize);
        if !same(have, want) {
            return Err(format!(
                "Filament {} was sliced for a {} mm nozzle, but the nozzle grouping prints it \
                 with a {} mm rack nozzle.; no G-code was written",
                filament + 1,
                size_text(want),
                size_text(have)
            )
            .into());
        }
    }
    Ok(())
}

/// Refuses a grouping that puts one extruder's filaments on rack nozzles of
/// two sizes. The reference writes the file with a warning
/// (GCode.cpp l.5142 `rack_group_mixes_nozzle_diameters`, its own words
/// below): the H2C's firmware refuses to change between rack hotends of
/// different sizes in a print, so the file would stop the printer at the
/// first such change. Refused here instead, as G-code goes to a real
/// printer (owner: "the rack_group_mixes_nozzle_diameters refusal").
pub(crate) fn check_one_size(settings: &Settings, result: &GroupResult) -> Result<(), Failure> {
    let counts = counts(settings);
    if counts.len() != 2 {
        return Ok(());
    }
    for (extruder, count) in counts.iter().enumerate() {
        if *count < 2 {
            continue;
        }
        let mut sizes: Vec<f64> = Vec::new();
        for filament in &result.used {
            let Some(nozzle) = result.nozzle_for(i64::from(*filament)) else {
                continue;
            };
            if nozzle.extruder_id != extruder as i32 {
                continue;
            }
            if let Some(d) = positive(&nozzle.diameter)
                && !sizes.iter().any(|size| same(*size, d))
            {
                sizes.push(d);
            }
        }
        if sizes.len() >= 2 {
            return Err(format!(
                "This plate uses two nozzle diameters on the same extruder rack ({} mm on \
                 extruder {}). H2C firmware currently refuses to change nozzle diameter during \
                 a print. Split the plate by nozzle diameter or print the objects separately.; \
                 no G-code was written",
                sizes
                    .iter()
                    .map(|size| size_text(*size))
                    .collect::<Vec<_>>()
                    .join(" and "),
                extruder + 1
            )
            .into());
        }
    }
    Ok(())
}

/// ToolSlots.cpp `physical_nozzle_properties`, for a printer with a rack
/// set by hand of mixed sizes, or of nozzles of two sizes with a rack
/// (`emit_physical_nozzle_properties`): every nozzle, an extruder's rack
/// one by one, "0.4-standard,0.6-standard". None elsewhere, where the
/// writer's own (one to an extruder) serves.
pub(crate) fn physical_nozzles(settings: &Settings) -> Option<String> {
    let racks = racks(settings);
    let nozzles = numbers(settings, "nozzle_diameter");
    if racks.iter().all(Vec::is_empty) || nozzles.len() < 2 {
        return None;
    }
    let semm = settings
        .flag("single_extruder_multi_material")
        .ok()
        .flatten()
        == Some(true);
    let extruders_differ = !semm && nozzles.iter().any(|d| !same(*d, nozzles[0]));
    if !mixed_sizes(&racks) && !extruders_differ {
        return None;
    }
    let word = |kind: VolumeType| match kind {
        VolumeType::HIGH_FLOW => "high_flow",
        VolumeType::HYBRID => "hybrid",
        VolumeType::TPU_HIGH_FLOW => "tpu_high_flow",
        VolumeType::TPU_285 => "tpu285",
        VolumeType::E3D_HIGH_FLOW => "e3d_high_flow",
        _ => "standard",
    };
    let mut out = Vec::new();
    for (extruder, size) in nozzles.iter().enumerate() {
        match racks.get(extruder).filter(|rack| !rack.is_empty()) {
            Some(rack) => out.extend(
                rack.iter()
                    .map(|nozzle| format!("{}-{}", size_text(nozzle.diameter), word(nozzle.kind))),
            ),
            None => out.push(format!(
                "{}-{}",
                size_text(*size),
                word(extruder_kind(settings, extruder))
            )),
        }
    }
    Some(out.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn many(values: &[&str]) -> edge_config::Value {
        edge_config::Value::Many(values.iter().map(|v| (*v).to_owned()).collect())
    }

    fn h2c() -> Settings {
        let mut settings = Settings::from_json(
            r#"{"nozzle_diameter": ["0.4", "0.4"], "extruder_max_nozzle_count": ["1", "6"],
                "nozzle_volume_type": ["Standard", "Standard"],
                "filament_diameter": ["1.75", "1.75", "1.75"],
                "filament_map": ["2", "2", "1"]}"#,
        )
        .unwrap();
        settings.insert("filament_rack_nozzle_diameter", many(&["0"]));
        settings
    }

    #[test]
    fn rack_texts_are_read() {
        let rack = parse(" 0.4, 0.6:high_flow ,x, 0.8:odd,,");
        assert_eq!(rack.len(), 3);
        assert_eq!(rack[1].kind, VolumeType::HIGH_FLOW);
        assert!(rack[1].kind_given && !rack[2].kind_given);
        assert_eq!(rack[2].diameter, 0.8);
    }

    /// The project's text over the printer's over the machine's own.
    #[test]
    fn the_project_wins_over_the_printer_over_the_machine() {
        let mut settings = h2c();
        assert_eq!(
            text_of(&settings, 1),
            ("0.4,0.4,0.4,0.4,0.4,0.4".into(), None)
        );
        assert_eq!(text_of(&settings, 0).0, "");
        settings.insert("extruder_rack_nozzle_diameter", many(&["", "0.4,0.6"]));
        assert_eq!(racks(&settings)[1].len(), 2);
        settings.insert("extruder_rack_nozzle_override", many(&["", "0.4,0.4,0.8"]));
        let (text, key) = text_of(&settings, 1);
        assert_eq!(
            (text.as_str(), key),
            ("0.4,0.4,0.8", Some("extruder_rack_nozzle_override"))
        );
        assert!(mixed_sizes(&racks(&settings)));
        assert!(racks(&settings)[0].is_empty());
    }

    #[test]
    fn a_pinned_filament_prints_with_its_pins_size() {
        let mut settings = h2c();
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.8"]),
        );
        settings.insert("filament_rack_nozzle_diameter", many(&["0.6", "0", "0.8"]));
        let racks = racks(&settings);
        assert_eq!(filament_size(&settings, &racks, 0), 0.6);
        assert_eq!(filament_size(&settings, &racks, 1), 0.4);
        // On the left, whose has no rack: the pin does not apply.
        assert_eq!(filament_size(&settings, &racks, 2), 0.4);
        // A slot pin wins over a size pin: slot 4 is the 0.8.
        settings.insert("filament_rack_nozzle_slot", many(&["4", "0", "0"]));
        assert_eq!(filament_size(&settings, &racks, 0), 0.8);
        validate(&settings, &[0, 1, 2]).unwrap();
    }

    #[test]
    fn racks_the_engine_cannot_print_are_refused() {
        let mut settings = h2c();
        settings.insert("extruder_rack_nozzle_override", many(&["", "0.6,0.6"]));
        let said = validate(&settings, &[0]).unwrap_err().to_string();
        assert!(said.contains("0.4 mm is not one of its rack"), "{said}");
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.4:high_flow"]),
        );
        let said = validate(&settings, &[0]).unwrap_err().to_string();
        assert!(said.contains("mixed nozzle kinds"), "{said}");
        settings.insert("extruder_rack_nozzle_override", many(&["0.4,0.4", ""]));
        let said = validate(&settings, &[0]).unwrap_err().to_string();
        assert!(said.contains("lists 2 nozzles, but it holds 1"), "{said}");
        settings.insert("extruder_rack_nozzle_override", many(&["", "0.4,0.6"]));
        settings.insert("filament_rack_nozzle_diameter", many(&["0.8"]));
        let said = validate(&settings, &[0]).unwrap_err().to_string();
        assert!(said.contains("asks for a 0.8 mm rack nozzle"), "{said}");
        settings.insert("filament_rack_nozzle_diameter", many(&["0"]));
        settings.insert("filament_rack_nozzle_slot", many(&["0", "0", "2"]));
        let said = validate(&settings, &[2]).unwrap_err().to_string();
        assert!(said.contains("Left slot 2, but that rack has 0"), "{said}");
        assert!(check_pins(&settings, false).is_err());
        check_pins(&settings, true).unwrap();
    }

    fn nozzles(sizes: &[(&str, i32)]) -> Vec<NozzleInfo> {
        (sizes.iter().enumerate())
            .map(|(n, (diameter, extruder))| NozzleInfo {
                diameter: (*diameter).to_owned(),
                volume_type: VolumeType::STANDARD,
                extruder_id: *extruder,
                group_id: n as i32,
            })
            .collect()
    }

    #[test]
    fn filaments_go_to_nozzles_of_their_size_and_slot() {
        let mut settings = h2c();
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.6"]),
        );
        settings.insert("filament_rack_nozzle_diameter", many(&["0.6", "0", "0"]));
        settings.insert("filament_map", many(&["2", "2", "1"]));
        let list = nozzles(&[("0.4", 0), ("0.4", 1), ("0.6", 1), ("0.4", 1), ("0.6", 1)]);
        // The grouping put filament 1 on a 0.4 and filament 2 on a 0.6.
        let mut result = GroupResult::create(vec![3, 4, 0], list.clone(), vec![0, 1, 2]).unwrap();
        assert!(check(&settings, &result).is_err());
        assert_eq!(pin(&settings, &mut result), 2);
        assert_eq!(result.map, [2, 1, 0]);
        check(&settings, &result).unwrap();
        let said = check_one_size(&settings, &result).unwrap_err().to_string();
        assert!(said.contains("0.6 and 0.4 mm on extruder 2"), "{said}");
        // The file's list of every nozzle (h2c3_mix2_211_pn, the reference's).
        assert_eq!(
            physical_nozzles(&settings).unwrap(),
            "0.4-standard,0.4-standard,0.6-standard,0.4-standard,0.6-standard"
        );
        // Slot pins, on a rack of one size too.
        let mut settings = h2c();
        settings.insert("filament_rack_nozzle_slot", many(&["1", "1", "0"]));
        let list = nozzles(&[("0.4", 0), ("0.4", 1), ("0.4", 1), ("0.4", 1)]);
        let mut result = GroupResult::create(vec![3, 2, 0], list, vec![0, 1, 2]).unwrap();
        assert_eq!(pin(&settings, &mut result), 2);
        assert_eq!(result.map, [1, 1, 0]);
        check_one_size(&settings, &result).unwrap();
    }
}
