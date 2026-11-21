//! A Bambu Lab printer of two nozzles, one to each extruder (the H2D):
//! what its templates are told of which nozzle each filament prints from,
//! and what a change between filaments carries over from one nozzle to the
//! other (GCode.cpp `_do_export`, the "Ultra" placeholder blocks;
//! `set_extruder`, `append_tcr`, `cross_extruder_flush_volume`,
//! `incoming_extruder_retracted_length`, `set_ec_retraction_placeholders`).
//!
//! The grouping itself (which nozzle) is worked out before the writer
//! (edge-engine `grouping`; for copies printed one after another, the
//! plate's own map) and handed over in [`Plate::nozzles`]. On the H2C's
//! rack (one nozzle on the left, six on the right) a filament may print from
//! any nozzle of its extruder's rack: a change between two of them is a
//! rack nozzle change, and the nozzle taken from the rack is flushed of what
//! it last held ([`Held`]). On Bambu's tower the ramming before a change
//! onto another nozzle (`M632` / `M633` inside one extruder's rack) is laid
//! by `bambu_tower`.

use edge_template::{Value, Variables};

use crate::{GcodeError, Layer, NozzleMap, Plate, PrintSettings};

/// Whether a print on a Bambu Lab printer of `s.tools` nozzles can be
/// written: only with the grouping's answer at hand, on two nozzles.
pub(crate) fn check(s: &PrintSettings) -> Result<(), GcodeError> {
    if s.tools > 2 {
        return Err(GcodeError::Unsupported(format!(
            "a Bambu Lab printer of {} nozzles: only two nozzles (the H2D) are written",
            s.tools
        )));
    }
    if s.plate.nozzles.is_none() {
        return Err(GcodeError::Unsupported(
            "a Bambu Lab printer of two nozzles: which nozzle each filament prints from was not \
             worked out for this plate, so the nozzle changes cannot be written"
                .into(),
        ));
    }
    Ok(())
}

/// The print-wide names a two-nozzle printer's templates read, set over
/// [`crate::bambu_vars::single_nozzle`]'s (GCode.cpp `_do_export`):
/// `filament_map` from the grouping, and for each physical extruder the
/// first filament and the first filament not for support it prints, -1
/// for one that prints nothing (`first_filaments`,
/// `first_non_support_filaments`; Bambu Studio's
/// `match_physical_extruder_for_each_filament`), by
/// `physical_extruder_map`. `layers` are the writer's, in print order.
pub(crate) fn start(variables: &mut Variables, map: &NozzleMap, layers: &[Layer<'_>]) {
    let int = Value::Int;
    let filaments = variables
        .list("filament_colour")
        .map_or(0, <[Value]>::len)
        .max(1);
    let extruders: Vec<Value> = (0..filaments)
        .map(|filament| int(map.extruder(filament) as i64 + 1))
        .collect();
    variables.set_list("filament_map", extruders);
    let nozzles = 2;
    let support = |filament: usize| {
        variables
            .list("filament_is_support")
            .and_then(|list| list.get(filament))
            .is_some_and(|value| matches!(value.to_string().trim(), "1" | "true"))
    };
    let mut first = vec![-1i64; nozzles];
    let mut first_model = vec![-1i64; nozzles];
    for layer in layers {
        let extruder = map.extruder(layer.filament);
        if extruder >= nozzles {
            continue;
        }
        if first[extruder] < 0 {
            first[extruder] = layer.filament as i64;
        }
        if first_model[extruder] < 0 && !support(layer.filament) {
            first_model[extruder] = layer.filament as i64;
        }
    }
    // Logical extruder to physical (`physical_extruder_map`), itself
    // where the map is missing or out of range.
    let physical: Vec<usize> = (0..nozzles)
        .map(|extruder| {
            variables
                .list("physical_extruder_map")
                .and_then(|list| list.get(extruder))
                .and_then(|value| value.to_string().trim().parse::<usize>().ok())
                .filter(|p| *p < nozzles)
                .unwrap_or(extruder)
        })
        .collect();
    let to_physical = |logical: &[i64]| {
        let mut out = vec![int(-1); nozzles];
        for (extruder, filament) in logical.iter().enumerate() {
            out[physical[extruder]] = int(*filament);
        }
        out
    };
    variables.set_list("first_filaments", to_physical(&first));
    variables.set_list("first_non_support_filaments", to_physical(&first_model));
    by_nozzle_id(variables, map);
}

/// On a nozzle rack the templates index each nozzle's diameter and kind by
/// the nozzle's number (`nozzle_diameter_at_nozzle_id[next_nozzle_id]`;
/// GCode.cpp `get_nozzle_diameters_by_nozzle_id`,
/// `get_nozzle_volume_types_by_nozzle_id`), which reaches past the one
/// entry to each extruder `bambu_vars` gives: every nozzle a filament prints
/// from is given its extruder's. Nothing changes with one nozzle to each
/// extruder.
fn by_nozzle_id(variables: &mut Variables, map: &NozzleMap) {
    let Some(last) = map.nozzles.iter().copied().max().filter(|n| *n >= 0) else {
        return;
    };
    let count = last as usize + 1;
    for name in ["nozzle_diameter_at_nozzle_id", "nozzle_volume_types"] {
        let Some(list) = variables.list(name).map(<[Value]>::to_vec) else {
            continue;
        };
        if list.len() >= count || list.is_empty() {
            continue;
        }
        // Each nozzle's extruder, from a filament on it; a nozzle no
        // filament prints from is never asked for and keeps the last
        // extruder's.
        let mut extruder_of = vec![list.len() - 1; count];
        for (filament, nozzle) in map.nozzles.iter().enumerate() {
            if *nozzle >= 0 {
                extruder_of[*nozzle as usize] = map.extruder(filament);
            }
        }
        let values = (0..count)
            .map(|nozzle| {
                if nozzle < list.len() && map.nozzles.iter().all(|n| *n != nozzle as i32) {
                    return list[nozzle].clone();
                }
                list[extruder_of[nozzle].min(list.len() - 1)].clone()
            })
            .collect();
        variables.set_list(name, values);
    }
}

/// The nozzle a filament prints from, by number (`filament_nozzle_map`),
/// or its extruder's where the map names none. On the H2D the two are the
/// same; on the H2C's rack an extruder has several.
fn nozzle_of(map: &NozzleMap, filament: usize) -> usize {
    match map.nozzles.get(filament) {
        Some(nozzle) if *nozzle >= 0 => *nozzle as usize,
        _ => map.extruder(filament),
    }
}

/// Whether a change from `from` to `to` goes between two nozzles of one
/// extruder's rack (the H2C's): a rack nozzle change.
pub(crate) fn rack_change(map: &NozzleMap, from: usize, to: usize) -> bool {
    map.extruder(from) == map.extruder(to) && nozzle_of(map, from) != nozzle_of(map, to)
}

/// What a change between filaments on a printer of several nozzles keeps
/// track of: the filament each nozzle holds, and how far each extruder's
/// filament was left retracted when it was put away.
#[derive(Clone, Debug, Default)]
pub(crate) struct Held {
    filament: Vec<Option<usize>>,
    retracted: Vec<f64>,
}

impl Held {
    pub(crate) fn new() -> Self {
        Self {
            filament: vec![None; 2],
            retracted: vec![0.0; 2],
        }
    }

    /// The filament now printing is in its nozzle (GCode.cpp
    /// `note_filament_loaded`; on a rack, the nozzle of the extruder's
    /// rack it prints from).
    pub(crate) fn loaded(&mut self, map: &NozzleMap, filament: usize) {
        let nozzle = nozzle_of(map, filament);
        if self.filament.len() <= nozzle {
            self.filament.resize(nozzle + 1, None);
        }
        self.filament[nozzle] = Some(filament);
    }

    /// What a change from `from` to `to` flushes, in mm³, where the two
    /// print from different nozzles (GCode.cpp
    /// `cross_extruder_flush_volume`): what `to`'s nozzle still holds
    /// flushed into it, nothing where it holds nothing or `to` already.
    /// None for a change within one nozzle, which flushes as on a printer
    /// of one nozzle.
    ///
    /// On the H2C's rack this holds for a change onto another nozzle of
    /// the same extruder too: the nozzle taken from the rack still holds
    /// the filament it last printed, which is what the new one pushes out.
    /// The reference flushes there from the filament put away instead
    /// (Print.cpp `_make_wipe_tower`: its nozzle tracker,
    /// `HotendFilamentTracker`, runs on the H3C alone; GCode.cpp
    /// `append_tcr` zeroes only changes onto another extruder), which
    /// flushes a full colour change into a nozzle that already holds the
    /// new filament (measured: work/tower/h2c2, h2c_two_22_pn, two
    /// filaments each on a rack nozzle of its own: every change there is
    /// `M620.10 ... L82.7` (0 to 1) or `L265.7` (1 to 0), here `L0`, the
    /// nozzle taken holding the new filament or, at its first use,
    /// nothing, as on the H2D); not followed.
    pub(crate) fn cross_flush(
        &self,
        map: &NozzleMap,
        from: usize,
        to: usize,
        s: &PrintSettings,
    ) -> Option<f64> {
        let (old, new) = (nozzle_of(map, from), nozzle_of(map, to));
        if old == new {
            return None;
        }
        Some(match self.filament.get(new).copied().flatten() {
            Some(held) if held != to => flush_volume(s, map, held, to),
            _ => 0.0,
        })
    }

    /// How far the incoming extruder's filament is retracted, the
    /// outgoing extruder's left at `outgoing` (GCode.cpp
    /// `incoming_extruder_retracted_length`, Bambu Studio's
    /// `get_extruder_retracted_length`): one state to an extruder, shared
    /// by its filaments, 0 before its first use.
    pub(crate) fn swap_retraction(
        &mut self,
        map: &NozzleMap,
        from: Option<usize>,
        to: usize,
        outgoing: f64,
    ) -> f64 {
        if let Some(from) = from
            && let Some(slot) = self.retracted.get_mut(map.extruder(from))
        {
            *slot = outgoing;
        }
        self.retracted.get(map.extruder(to)).copied().unwrap_or(0.0)
    }
}

/// What goes from filament `from` to `to` flushes, in mm³, times the
/// multiplier, on the nozzle `to` prints from. A project with a matrix to
/// each nozzle (2 × n × n, as Bambu Studio writes an H2D project) is read
/// at that nozzle's; one of a single n × n matrix, at it for both.
///
/// The reference reads the list as one square matrix whatever its length
/// (GCode.cpp `set_extruder`, `cross_extruder_flush_volume`: n is the
/// square root of its length), so with a matrix to each nozzle it reads
/// the first nozzle's rows for both, and those out of place for more
/// than two filaments. That is a flaw and is not followed here; the two
/// agree on a project of one matrix.
pub(crate) fn flush_volume(s: &PrintSettings, map: &NozzleMap, from: usize, to: usize) -> f64 {
    let volumes = &s.flush_volumes;
    let filaments = s.filaments.max(1);
    let square = filaments * filaments;
    let (matrix, n) = if volumes.len() == 2 * square {
        let nozzle = map.extruder(to).min(1);
        (&volumes[nozzle * square..(nozzle + 1) * square], filaments)
    } else {
        let n = (volumes.len() as f64).sqrt().round() as usize;
        (&volumes[..], n)
    };
    if from >= n || to >= n {
        return 0.0;
    }
    matrix.get(from * n + to).copied().unwrap_or(0.0)
}

/// The extruder-change long retraction of `filament`, as the change
/// G-code reads it on its `M620.11 K.. R..` line (GCode.cpp
/// `set_ec_retraction_placeholders`): `long_retractions_when_ec` and
/// `retraction_distances_when_ec`, an entry that is `nil` or missing
/// read as off, and no distance with it off.
pub(crate) fn set_ec_retraction(variables: &mut Variables, filament: usize) {
    let entry = |variables: &Variables, name: &str| -> Option<String> {
        let list = variables.list(name)?;
        let value = list.get(filament).or(list.last())?.to_string();
        let value = value.trim().to_owned();
        (value != "nil" && !value.is_empty()).then_some(value)
    };
    let long = entry(variables, "long_retractions_when_ec")
        .is_some_and(|value| value != "0" && value != "false");
    let distance = entry(variables, "retraction_distances_when_ec")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && long)
        .unwrap_or(0.0);
    variables.set("long_retraction_when_ec", Value::Bool(long));
    variables.set("retraction_distance_when_ec", Value::Float(distance));
}

/// The long retraction when the filament is cut of `filament`, as the
/// change G-code reads it (GCode.cpp `set_cut_retraction_placeholders`):
/// its `retraction_distances_when_cut`, and `long_retractions_when_cut`
/// where that distance is above 0 (the list is the effective one already,
/// `settings.rs`). Set for the first filament and after each change, so a
/// change G-code reads the outgoing filament's.
pub(crate) fn set_cut_retraction(variables: &mut Variables, filament: usize) {
    let entry = |variables: &Variables, name: &str| -> Option<String> {
        let list = variables.list(name)?;
        Some(
            list.get(filament)
                .or(list.last())?
                .to_string()
                .trim()
                .to_owned(),
        )
    };
    let distance = entry(variables, "retraction_distances_when_cut")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(0.0);
    let long = entry(variables, "long_retractions_when_cut")
        .is_some_and(|value| value != "0" && value != "false" && value != "nil")
        && distance > 0.0;
    variables.set("retraction_distance_when_cut", Value::Float(distance));
    variables.set("long_retraction_when_cut", Value::Bool(long));
}

/// The grouping's maps as the file's settings list them, in place of the
/// project's (GCode.cpp `append_full_config`, `grouping_header_maps`):
/// `filament_map`, `filament_nozzle_map` and `filament_volume_map`.
pub(crate) fn listed(plate: &Plate, key: &str) -> Option<String> {
    let map = plate.nozzles.as_ref()?;
    let join = |values: &mut dyn Iterator<Item = String>| values.collect::<Vec<_>>().join(",");
    match key {
        "filament_map" => Some(join(&mut map.extruders.iter().map(u32::to_string))),
        "filament_nozzle_map" => Some(join(&mut map.nozzles.iter().map(i32::to_string))),
        "filament_volume_map" => Some(join(&mut map.volumes.iter().map(i32::to_string))),
        _ => None,
    }
}

/// Whether `key` is one of the idle-nozzle pre-heating settings and the
/// printer pre-heats: the reference lists them in the file's settings only
/// then (GCode.cpp `append_full_config`, `dump_pre_heating_keys`).
pub(crate) fn listed_pre_heating(settings: &edge_config::Settings, key: &str) -> bool {
    const KEYS: [&str; 3] = [
        "enable_pre_heating",
        "filament_pre_cooling_temperature",
        "filament_preheat_temperature_delta",
    ];
    KEYS.contains(&key) && settings.flag("enable_pre_heating").ok().flatten() == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rack_nozzle_is_known_by_its_number_in_the_templates_lists() {
        let mut variables = Variables::default();
        variables.set_list(
            "nozzle_diameter_at_nozzle_id",
            vec![Value::text("0.4"), Value::text("0.6")],
        );
        variables.set_list("nozzle_volume_types", vec![Value::Int(0), Value::Int(1)]);
        // The left nozzle and nozzles 2 and 4 of the right extruder's rack.
        let map = NozzleMap {
            extruders: vec![1, 2, 2],
            nozzles: vec![0, 4, 2],
            volumes: vec![0, 1, 1],
        };
        by_nozzle_id(&mut variables, &map);
        let diameters: Vec<String> = (variables.list("nozzle_diameter_at_nozzle_id").unwrap())
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(diameters, ["0.4", "0.6", "0.6", "0.6", "0.6"]);
        assert_eq!(variables.list("nozzle_volume_types").unwrap().len(), 5);
        // One nozzle to each extruder (the H2D): nothing changes.
        let mut two = Variables::default();
        two.set_list("nozzle_diameter_at_nozzle_id", vec![Value::text("0.4"); 2]);
        let h2d = NozzleMap {
            extruders: vec![1, 2],
            nozzles: vec![0, 1],
            volumes: vec![0, 0],
        };
        by_nozzle_id(&mut two, &h2d);
        assert_eq!(two.list("nozzle_diameter_at_nozzle_id").unwrap().len(), 2);
    }
}
