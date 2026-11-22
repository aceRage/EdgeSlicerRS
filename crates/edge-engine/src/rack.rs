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
//! yet), a pin no extruder can take on an automatic filament map
//! ([`check_automatic`]), and a grouping that puts one extruder's
//! filaments on rack nozzles of two sizes ([`check_one_size`]), which the
//! reference only warns of.
//!
//! Two rules differ from the pinned reference on purpose, both its errors
//! (work/cppfix/pin/RULE.md, shared with the C++ fix, PR 1163):
//! filaments pinned to a size spread over free nozzles of it ([`pin`],
//! rule S), and a pin is honoured on an automatic filament map
//! ([`settle_automatic`], [`pin_bars`], rules P and P2: filaments of
//! another size kept off a rack its pins put on one size).

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

/// Whether `extruder` shows rack slot `slot` (from 1): it holds more than
/// one nozzle and its rack lists at least two and at least `slot`
/// (ToolSlots.cpp `rack_slot_count`).
fn has_slot(settings: &Settings, racks: &[Vec<RackNozzle>], extruder: usize, slot: usize) -> bool {
    let holds = counts(settings).get(extruder).copied().unwrap_or(1);
    let listed = racks.get(extruder).map_or(0, Vec::len);
    holds > 1 && listed >= 2 && listed >= slot
}

/// The extruders (from 0) that can take a pinned filament's pin, none
/// where it is not pinned (work/cppfix/pin/RULE.md, "Which extruders can
/// take a pin"): for a slot pin, the one extruder the slot belongs to (the
/// saved extruder where it shows that slot, else the lowest that does);
/// for a size pin, every extruder with a nozzle of that size (in its rack,
/// or its own nozzle where it has no rack).
pub(crate) fn pin_extruders(
    settings: &Settings,
    racks: &[Vec<RackNozzle>],
    filament: usize,
) -> Option<Vec<usize>> {
    let nozzles = numbers(settings, "nozzle_diameter");
    let slot = slot_pin(settings, filament);
    if slot > 0 {
        let saved = extruder_of(settings, filament);
        let owner = if has_slot(settings, racks, saved, slot) {
            Some(saved)
        } else {
            (0..nozzles.len()).find(|e| has_slot(settings, racks, *e, slot))
        };
        return Some(owner.into_iter().collect());
    }
    let size = numbers(settings, "filament_rack_nozzle_diameter")
        .get(filament)
        .copied()
        .filter(|pin| *pin > 0.0)?;
    Some(
        (0..nozzles.len())
            .filter(|e| match racks.get(*e).filter(|rack| !rack.is_empty()) {
                Some(rack) => rack.iter().any(|nozzle| same(nozzle.diameter, size)),
                None => same(nozzles[*e], size),
            })
            .collect(),
    )
}

/// Whether the filament map is worked out by the grouping (any
/// `filament_map_mode` but "Manual"; the reference's default is
/// "Auto For Flush", `reference_defaults`).
pub(crate) fn automatic(settings: &Settings) -> bool {
    settings
        .text("filament_map_mode")
        .unwrap_or("Auto For Flush")
        != "Manual"
}

/// Rule P, before the walls are laid: on an automatic filament map, a
/// pinned filament whose saved extruder (`filament_map`) cannot take its
/// pin is put on the lowest extruder that can, in `filament_map`, so the
/// size it is sliced for (`filament_size`, `crate::nozzle_process`) and
/// every later reading of its pin go by that extruder, its "pin extruder"
/// (RULE.md); the grouping then bars the extruders that cannot take it
/// ([`pin_bars`], with rule P2). A plate printed object by object reads
/// this map as set by hand, the pin bars moving a used filament off an
/// extruder they keep it off (`crate::grouping::group`); this map is
/// settled for unused pinned filaments too, where RULE.md leaves their
/// entries. Unchanged where no extruder has a rack, on a manual map and on
/// a printer of fewer than two extruders.
///
/// On purpose not the pinned reference: it reads the pin through the
/// project's `filament_map` as saved, not the map the grouping works out
/// (ToolSlots.cpp l.497 `filament_rack_pin`, l.196 `physical_extruder_of`;
/// the grouping, ToolOrdering.cpp l.1985, knows nothing of pins), so a
/// pinned filament the grouping moves loses its pin without a word
/// (measured on h2c3_mix2_auto_pn: filament 1 pinned to 0.6, saved map
/// 1,2,1, printed from a 0.4 nozzle, map 1,0,1). A C++ fix PR is being
/// opened for it (aceRage/EdgeSlicer, PR 1163).
pub(crate) fn settle_automatic(settings: &mut Settings) {
    let racks = racks(settings);
    let extruders = numbers(settings, "nozzle_diameter").len();
    if !automatic(settings) || racks.iter().all(Vec::is_empty) || extruders < 2 {
        return;
    }
    let mut map: Vec<usize> = crate::nozzle_process::extruders_of(settings, extruders);
    let mut changed = false;
    for (filament, extruder) in map.iter_mut().enumerate() {
        let Some(allowed) = pin_extruders(settings, &racks, filament) else {
            continue;
        };
        if !allowed.contains(extruder)
            && let Some(&lowest) = allowed.first()
        {
            *extruder = lowest;
            changed = true;
        }
    }
    if changed {
        settings.insert(
            "filament_map",
            edge_config::Value::Many(map.iter().map(|e| (e + 1).to_string()).collect()),
        );
    }
}

/// Rule P's errors before the grouping, for the filaments the plate prints
/// (`used`, from 0) on an automatic map (RULE.md E1), and what this engine
/// cannot do besides: a pin to a size other than its extruder's own that
/// more than one extruder could take (the process its walls are laid with
/// would be settled only by the grouping; no printer this is done for has
/// two racks).
pub(crate) fn check_automatic(settings: &Settings, used: &[u32]) -> Result<(), Failure> {
    let racks = racks(settings);
    if !automatic(settings) || racks.iter().all(Vec::is_empty) || racks.len() < 2 {
        return Ok(());
    }
    let nozzles = numbers(settings, "nozzle_diameter");
    for filament in used {
        let f = *filament as usize;
        let Some(allowed) = pin_extruders(settings, &racks, f) else {
            continue;
        };
        let slot = slot_pin(settings, f);
        // A slot no extruder shows: `validate`'s own message on the pin
        // extruder (the saved one here), as RULE.md has it.
        if allowed.is_empty() && slot > 0 {
            continue;
        }
        if allowed.is_empty() {
            return Err(format!(
                "Filament {} asks for a {} mm rack nozzle, but no extruder has one. \
                 (filament_rack_nozzle_diameter); no G-code was written",
                f + 1,
                size_text(size_pin(settings, &racks, f))
            )
            .into());
        }
        let pin = size_pin(settings, &racks, f);
        let own = |e: usize| nozzles.get(e).copied().unwrap_or(0.0);
        if allowed.len() > 1 && allowed.iter().any(|e| !same(own(*e), pin)) {
            return Err(format!(
                "Filament {} is pinned to a {} mm rack nozzle, which more than one extruder has, \
                 on an automatic filament map: which process its walls are laid with would only \
                 be settled by the grouping; set the filaments' nozzles by hand (Manual); no \
                 G-code was written",
                f + 1,
                size_text(pin)
            )
            .into());
        }
    }
    Ok(())
}

/// Rules P and P2 in the grouping (work/cppfix/pin/RULE.md): for each
/// extruder, the used filaments (from 0) barred from it, over `limits`, the
/// filaments it cannot print already (from 0). None on a manual map, with
/// fewer than two extruders, or without a rack.
///
/// - P: a used pinned filament is barred from every extruder that cannot
///   take its pin; one barred from all (with `limits`) is refused (E2).
/// - P2 (owner, 2026-11-22: "keep them off"): an extruder whose pinned
///   filaments (by their pin extruder) are all of one size d keeps off it
///   every other used filament that would print there with another size
///   (a pinned one its pinned size, any other the extruder's own), unless
///   that would leave it no extruder; then it is left free and the plate is
///   refused for two sizes on one rack ([`check_one_size`]) if the grouping
///   puts it there. The H2C's firmware will not change between rack
///   nozzles of two sizes in a print, so this keeps a pin on an automatic
///   map printable.
///
/// Both differ from the pinned reference on purpose; see
/// [`settle_automatic`].
pub(crate) fn pin_bars(
    settings: &Settings,
    used: &[u32],
    extruders: usize,
    limits: &[std::collections::BTreeSet<i32>],
) -> Result<Vec<std::collections::BTreeSet<u32>>, Failure> {
    let mut bars = vec![std::collections::BTreeSet::new(); extruders];
    let racks = racks(settings);
    if !automatic(settings) || racks.iter().all(Vec::is_empty) || extruders < 2 {
        return Ok(bars);
    }
    let limited = |bars: &[std::collections::BTreeSet<u32>], e: usize, f: u32| {
        bars[e].contains(&f) || limits.get(e).is_some_and(|set| set.contains(&(f as i32)))
    };
    // P.
    let mut pinned: Vec<(u32, usize, f64)> = Vec::new();
    for filament in used {
        let f = *filament as usize;
        let Some(allowed) = pin_extruders(settings, &racks, f) else {
            continue;
        };
        for (extruder, bar) in bars.iter_mut().enumerate() {
            if !allowed.contains(&extruder) {
                bar.insert(*filament);
            }
        }
        if (0..extruders).all(|e| limited(&bars, e, *filament)) {
            let slot = slot_pin(settings, f);
            return Err(if slot > 0 {
                format!(
                    "Filament {} is pinned to rack slot {slot}, but no extruder with that slot \
                     can print it.; no G-code was written",
                    f + 1
                )
            } else {
                format!(
                    "Filament {} is pinned to a {} mm rack nozzle, but no extruder that has one \
                     can print it.; no G-code was written",
                    f + 1,
                    size_text(size_pin(settings, &racks, f))
                )
            }
            .into());
        }
        pinned.push((
            *filament,
            extruder_of(settings, f),
            size_pin(settings, &racks, f),
        ));
    }
    // P2.
    let nozzles = numbers(settings, "nozzle_diameter");
    for extruder in 0..extruders {
        let mut sizes: Vec<f64> = Vec::new();
        for (_, own, size) in &pinned {
            if *own == extruder && !sizes.iter().any(|d| same(*d, *size)) {
                sizes.push(*size);
            }
        }
        let [d] = sizes[..] else {
            continue;
        };
        for filament in used {
            if limited(&bars, extruder, *filament) {
                continue;
            }
            let size = (pinned.iter())
                .find(|(f, _, _)| f == filament)
                .map_or_else(|| nozzles.get(extruder).copied().unwrap_or(0.0), |p| p.2);
            if same(size, d) {
                continue;
            }
            if (0..extruders).all(|e| e == extruder || limited(&bars, e, *filament)) {
                continue;
            }
            bars[extruder].insert(*filament);
        }
    }
    Ok(bars)
}

/// A filament pinned to a rack size or slot on an automatic filament map,
/// on a printer without a nozzle rack, is refused as before: there is no
/// rack nozzle to pin it to.
pub(crate) fn check_pins(settings: &Settings, manual: bool) -> Result<(), Failure> {
    let rackless = racks(settings).iter().all(Vec::is_empty);
    if !manual && rackless && any_pins(settings) {
        return Err(
            "a filament is pinned to a rack nozzle size or slot (filament_rack_nozzle_diameter, \
             filament_rack_nozzle_slot), but no extruder of this printer has a nozzle rack; \
             clear the pin; no G-code was written"
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

/// The size `filament` (from 0) prints with on `extruder` (from 0): the
/// rack size it is pinned to where that extruder's rack has it, else the
/// extruder's own ([`filament_size`] for the extruder the grouping put it
/// on, which on an automatic map is not the saved `filament_map`'s).
fn size_on(
    settings: &Settings,
    racks: &[Vec<RackNozzle>],
    filament: usize,
    extruder: usize,
) -> f64 {
    let nozzles = numbers(settings, "nozzle_diameter");
    let own = nozzles.get(extruder).copied().unwrap_or(0.0);
    let Some(rack) = racks.get(extruder).filter(|rack| !rack.is_empty()) else {
        return own;
    };
    let slot = slot_pin(settings, filament);
    let pin = if slot > 0 {
        rack.get(slot - 1).map_or(0.0, |nozzle| nozzle.diameter)
    } else {
        numbers(settings, "filament_rack_nozzle_diameter")
            .get(filament)
            .copied()
            .unwrap_or(0.0)
    };
    if pin <= 0.0 {
        return own;
    }
    rack.iter()
        .find(|nozzle| same(nozzle.diameter, pin))
        .map_or(own, |nozzle| nozzle.diameter)
}

/// ToolSlots.cpp `pin_rack_nozzle_diameters`: a used filament pinned to a
/// slot goes to that slot's nozzle of the extruder the grouping put it on;
/// on a rack of mixed sizes, one on a nozzle of another size than it is
/// sliced for goes to a nozzle of its extruder of that size. Returns how
/// many moved.
///
/// Where a filament pinned to a size goes differs from the pinned
/// reference on purpose (owner: "we should spread the pinned filaments
/// sharing a nozzle one if another one is unused"; rule S of
/// work/cppfix/pin/RULE.md, to the letter): the reference moves it to the
/// FIRST nozzle of its size (ToolSlots.cpp l.619-669), so two filaments
/// pinned to 0.6 share one 0.6 nozzle and flush into each other though
/// another 0.6 stands unused (h2c3_mix4_2211_pn: map 2,2,0,0 with slot 4 a
/// free 0.6). A C++ fix PR is being opened for it (aceRage/EdgeSlicer,
/// PR 1163).
///
/// Pass A, used filaments in order: a slot pin to its slot; on a rack of
/// mixed sizes, a filament already on its size stays (but one pinned to a
/// size, on the nozzle a lower size-pinned filament stayed on, waits for
/// pass B: RULE.md step 3b), one not pinned to a size goes to the first
/// nozzle of its size (of its current nozzle's kind where there is one),
/// as the reference; one pinned to a size waits for pass B. Pass B, those
/// in order: of the nozzles of its extruder of its size, the one with the
/// least (other kind, in use, not its current one, number), "in use" being
/// given to a used filament other than the ones of pass B not placed yet.
/// So a free nozzle of the size first, the lowest slot; when every one is
/// in use, the one it is on (3b) or else the lowest (shared).
pub(crate) fn pin(settings: &Settings, result: &mut GroupResult) -> usize {
    let racks = racks(settings);
    let slot_pins = has_slot_pins(settings);
    let mixed = mixed_sizes(&racks);
    if !mixed && !slot_pins {
        return 0;
    }
    let now_of = |result: &GroupResult, f: usize| -> Option<(i32, NozzleInfo)> {
        let current = *result.map.get(f)?;
        let now = usize::try_from(current)
            .ok()
            .and_then(|n| result.nozzles.get(n))
            .cloned()?;
        Some((current, now))
    };
    let of_size = |result: &GroupResult, extruder: i32, want: f64| -> Vec<usize> {
        (0..result.nozzles.len())
            .filter(|n| {
                let nozzle = &result.nozzles[*n];
                nozzle.extruder_id == extruder
                    && positive(&nozzle.diameter).is_some_and(|d| same(d, want))
            })
            .collect()
    };
    let mut moved = 0;
    let mut movers = Vec::new();
    let mut stayed: Vec<i32> = Vec::new();
    for filament in result.used.clone() {
        let f = filament as usize;
        let Some((current, now)) = now_of(result, f) else {
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
        let want = size_on(settings, &racks, f, now.extruder_id.max(0) as usize);
        let pinned = size_pin(settings, &racks, f) > 0.0;
        if positive(&now.diameter).is_some_and(|have| same(have, want)) {
            // 3b: a size-pinned filament on the nozzle a lower one pinned to
            // a size stayed on moves too, if a nozzle of its size is free.
            if pinned && stayed.contains(&current) {
                movers.push(f);
            } else if pinned {
                stayed.push(current);
            }
            continue;
        }
        if pinned {
            movers.push(f);
            continue;
        }
        let candidates = of_size(result, now.extruder_id, want);
        let target = (candidates.iter())
            .find(|n| result.nozzles[**n].volume_type == now.volume_type)
            .or(candidates.first());
        if let Some(target) = target {
            result.map[f] = *target as i32;
            moved += 1;
        }
    }
    for (place, f) in movers.iter().copied().enumerate() {
        let Some((current, now)) = now_of(result, f) else {
            continue;
        };
        let want = size_on(settings, &racks, f, now.extruder_id.max(0) as usize);
        let waiting = &movers[place..];
        let in_use = |n: usize| {
            (result.used.iter())
                .map(|other| *other as usize)
                .filter(|other| !waiting.contains(other))
                .any(|other| result.map.get(other) == Some(&(n as i32)))
        };
        let target = of_size(result, now.extruder_id, want)
            .into_iter()
            .min_by_key(|n| {
                (
                    result.nozzles[*n].volume_type != now.volume_type,
                    in_use(*n),
                    *n as i32 != current,
                    *n,
                )
            });
        if let Some(target) = target {
            result.map[f] = target as i32;
            if target as i32 != current {
                moved += 1;
            }
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
        // A pin on an automatic map is honoured where there is a rack
        // (rule P); refused only on a printer with none.
        check_pins(&settings, false).unwrap();
        settings.insert("extruder_max_nozzle_count", many(&["1", "1"]));
        settings.insert("extruder_rack_nozzle_override", many(&["", ""]));
        let said = check_pins(&settings, false).unwrap_err().to_string();
        assert!(
            said.contains("no extruder of this printer has a nozzle rack"),
            "{said}"
        );
        check_pins(&settings, true).unwrap();
    }

    /// Rule P (work/cppfix/pin/RULE.md): which extruders can take a pin,
    /// the pin extruder put into the map on an automatic map, E1, and the
    /// extruders barred in the grouping.
    #[test]
    fn a_pin_on_an_automatic_map_goes_to_an_extruder_that_can_take_it() {
        let mut settings = h2c();
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.6"]),
        );
        settings.insert("filament_map", many(&["1", "1", "1"]));
        settings.insert("filament_rack_nozzle_diameter", many(&["0.6", "0.4", "0"]));
        let racks = racks(&settings);
        assert_eq!(pin_extruders(&settings, &racks, 0), Some(vec![1]));
        // 0.4: the left's own nozzle and in the right's rack.
        assert_eq!(pin_extruders(&settings, &racks, 1), Some(vec![0, 1]));
        assert_eq!(pin_extruders(&settings, &racks, 2), None);
        // A slot belongs to the one extruder that shows it.
        settings.insert("filament_rack_nozzle_slot", many(&["0", "0", "4"]));
        assert_eq!(pin_extruders(&settings, &racks, 2), Some(vec![1]));
        // A manual map is left as it is.
        let mut manual = settings.clone();
        manual.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        settle_automatic(&mut manual);
        assert_eq!(manual.texts("filament_map").unwrap(), ["1", "1", "1"]);
        let bars = pin_bars(&manual, &[0, 1, 2], 2, &[]).unwrap();
        assert!(bars.iter().all(std::collections::BTreeSet::is_empty));
        // Automatic (the default): filaments 1 and 3 can only go right, 2
        // keeps its saved extruder, which can take it.
        settle_automatic(&mut settings);
        assert_eq!(settings.texts("filament_map").unwrap(), ["2", "1", "2"]);
        assert_eq!(filament_size(&settings, &racks, 0), 0.6);
        // P bars 1 and 3 from the left; P2: the right's pins are all 0.6,
        // so filament 2 (pinned 0.4, its pin extruder the left) is kept off
        // it; the left's are its own 0.4, which bars nothing more.
        let bars = pin_bars(&settings, &[0, 1, 2], 2, &[]).unwrap();
        assert_eq!(bars, [[0, 2].into(), [1].into()]);
        check_automatic(&settings, &[0, 1, 2]).unwrap();
        validate(&settings, &[0, 1, 2]).unwrap();
        // E1: no extruder can take the pin.
        settings.insert("filament_rack_nozzle_diameter", many(&["0.8", "0", "0"]));
        let said = check_automatic(&settings, &[0]).unwrap_err().to_string();
        assert!(
            said.starts_with("Filament 1 asks for a 0.8 mm rack nozzle, but no extruder has one."),
            "{said}"
        );
        settings.insert("filament_rack_nozzle_slot", many(&["0", "0", "6"]));
        // A slot no extruder shows: the existing message, on the pin
        // extruder (RULE.md, CHANGED).
        check_automatic(&settings, &[2]).unwrap();
        let said = validate(&settings, &[2]).unwrap_err().to_string();
        assert!(
            said.starts_with("Filament 3 is set to print on Right slot 6, but that rack has 4"),
            "{said}"
        );
        // An unused filament's pin is not checked.
        check_automatic(&settings, &[1]).unwrap();
    }

    /// Rule P2 (RULE.md, its ambiguity example and example 4): an extruder
    /// the pins put on one size keeps the filaments of another size off,
    /// unless they could go nowhere else.
    #[test]
    fn filaments_of_another_size_are_kept_off_a_pinned_rack() {
        let mut settings = h2c();
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.6"]),
        );
        settings.insert("filament_rack_nozzle_diameter", many(&["0.6", "0.4", "0"]));
        let all = [0, 1, 2];
        // Saved map 2,2,1: filament 2's pin extruder is the right, whose
        // pins are then of two sizes: P2 adds nothing.
        settings.insert("filament_map", many(&["2", "2", "1"]));
        settle_automatic(&mut settings);
        let bars = pin_bars(&settings, &all, 2, &[]).unwrap();
        assert_eq!(bars, [[0].into(), [].into()]);
        // Saved 2,1,1: the right is all 0.6; filaments 2 (0.4) and 3 (the
        // right's own 0.4) are kept off it.
        settings.insert("filament_map", many(&["2", "1", "1"]));
        let bars = pin_bars(&settings, &all, 2, &[]).unwrap();
        assert_eq!(bars, [[0].into(), [1, 2].into()]);
        // Filament 3 cannot print on the left: it is left free (the
        // grouping may then put two sizes on the rack, refused later).
        let limits = [std::collections::BTreeSet::from([2]), Default::default()];
        let bars = pin_bars(&settings, &all, 2, &limits).unwrap();
        assert_eq!(bars, [[0].into(), [1].into()]);
        // E2: filament 1 can only take the right, which it cannot print on.
        let limits = [Default::default(), std::collections::BTreeSet::from([0])];
        let said = pin_bars(&settings, &all, 2, &limits)
            .unwrap_err()
            .to_string();
        assert!(
            said.starts_with(
                "Filament 1 is pinned to a 0.6 mm rack nozzle, but no extruder that has one can \
                 print it."
            ),
            "{said}"
        );
        // A manual map: nothing.
        settings.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        let bars = pin_bars(&settings, &all, 2, &[]).unwrap();
        assert!(bars.iter().all(std::collections::BTreeSet::is_empty));
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

    fn four_filaments(sizes: &[&str], slots: &[&str]) -> Settings {
        let mut settings = h2c();
        settings.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.6"]),
        );
        settings.insert("filament_diameter", many(&["1.75"; 4]));
        settings.insert("filament_map", many(&["2", "2", "2", "1"]));
        settings.insert("filament_rack_nozzle_diameter", many(sizes));
        settings.insert("filament_rack_nozzle_slot", many(slots));
        settings
    }

    /// Rule S (work/cppfix/pin/RULE.md, its worked examples 1-3 on the
    /// rack 0.4,0.6,0.4,0.6: nozzles 0 left, 1-4 the right's slots).
    fn spread(sizes: &[&str], slots: &[&str], map: Vec<i32>) -> Vec<i32> {
        let settings = four_filaments(sizes, slots);
        let list = nozzles(&[("0.4", 0), ("0.4", 1), ("0.6", 1), ("0.4", 1), ("0.6", 1)]);
        let used = (0..map.len() as u32).collect();
        let mut result = GroupResult::create(map, list, used).unwrap();
        pin(&settings, &mut result);
        check(&settings, &result).unwrap();
        result.map
    }

    #[test]
    fn filaments_pinned_to_a_size_spread_over_its_free_nozzles() {
        let none = ["0"; 4];
        // 1: two pinned to 0.6 with two free 0.6 nozzles: one to each (the
        // reference: both on the first).
        let six = ["0.6", "0.6", "0", "0"];
        assert_eq!(spread(&six, &none, vec![1, 3, 0, 0]), [2, 4, 0, 0]);
        // 2: three pinned, two nozzles: the third shares the lowest.
        let six3 = ["0.6", "0.6", "0.6", "0"];
        assert_eq!(spread(&six3, &none, vec![1, 1, 3, 0]), [2, 4, 2, 0]);
        // 3: one already on its size stays; the other to the free one.
        assert_eq!(spread(&six, &none, vec![4, 1, 0, 0]), [4, 2, 0, 0]);
        // A slot pin is placed first and counts as in use, whatever order.
        let size2 = ["0", "0.6", "0", "0"];
        assert_eq!(
            spread(&size2, &["2", "0", "0", "0"], vec![1, 3, 0, 0]),
            [2, 4, 0, 0]
        );
        let size1 = ["0.6", "0", "0", "0"];
        assert_eq!(
            spread(&size1, &["0", "2", "0", "0"], vec![1, 1, 0, 0]),
            [4, 2, 0, 0]
        );
        // A slot pin keeps its slot even where it shares.
        assert_eq!(
            spread(&size1, &["0", "2", "0", "0"], vec![2, 1, 0, 0]),
            [2, 2, 0, 0]
        );
        // A filament still to move does not hold its nozzle: filament 1
        // takes slot 4, which filament 2 (pinned to 0.4) leaves.
        let mixed = ["0.6", "0.4", "0.6", "0"];
        assert_eq!(spread(&mixed, &none, vec![1, 4, 2, 0]), [4, 1, 2, 0]);
        // 3b (RULE.md, h2c4_onsize_2211's C++ grouping): two pinned to 0.6
        // left on one 0.6 by the grouping: the second to the free 0.6.
        assert_eq!(spread(&six, &none, vec![2, 2, 0, 0]), [2, 4, 0, 0]);
        // ... and stays where no other 0.6 is free.
        assert_eq!(spread(&six3, &none, vec![2, 2, 4, 0]), [2, 2, 4, 0]);
        // A filament not pinned goes to the first nozzle of its size, in use
        // or not, as the reference.
        assert_eq!(spread(&none, &none, vec![1, 2, 0, 0]), [1, 1, 0, 0]);
    }
}
