//! How long a run of filament changes takes (MultiNozzleUtils.cpp
//! `simulate_filament_change_time`), which the grouping adds to the flush
//! when it scores a way of putting filaments on nozzles.

use std::collections::{HashMap, HashSet};

use crate::nozzle::NozzleInfo;

/// Load and unload times of a filament change, in seconds: the whole way
/// from the spool (`standard_*`) and the last part from the selector
/// (`selector_*`) (`FilamentChangeTimeParams`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ChangeTimes {
    pub selector_load: f32,
    pub selector_unload: f32,
    pub standard_load: f32,
    pub standard_unload: f32,
}

impl ChangeTimes {
    /// From `machine_load_filament_time` and `machine_unload_filament_time`,
    /// the selector's part taken as half (ToolOrdering.cpp
    /// `build_filament_group_context`).
    pub fn from_machine(load: f64, unload: f64) -> Self {
        Self {
            standard_load: load as f32,
            standard_unload: unload as f32,
            selector_load: (load / 2.0) as f32,
            selector_unload: (unload / 2.0) as f32,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Location {
    Spool,
    Selector,
    Extruder,
}

/// How long the changes `changes` take, each a filament and the nozzle it
/// is loaded into, starting with every filament on its spool and every
/// extruder empty. `filaments` are the plate's filaments and `spool_group`
/// for each the spool group it is fed from (here its extruder);
/// `preload` says for each group whether it can bring the next filament
/// up while another prints (none here).
pub fn simulate_change_time(
    filaments: &[i32],
    nozzles: &[NozzleInfo],
    changes: &[(i32, i32)],
    spool_group: &[i32],
    times: &ChangeTimes,
    preload: &[bool],
) -> f64 {
    if filaments.is_empty() || nozzles.is_empty() || changes.is_empty() {
        return 0.0;
    }
    let load_spool_to_selector = times.standard_load - times.selector_load;
    let unload_spool_to_selector = times.standard_unload - times.selector_unload;
    let load_selector_to_extruder = times.selector_load;
    let unload_extruder_to_selector = times.selector_unload;

    let extruder_of: HashMap<i32, i32> = (nozzles.iter())
        .map(|nozzle| (nozzle.group_id, nozzle.extruder_id))
        .collect();
    let group_of: HashMap<i32, i32> = (filaments.iter().zip(spool_group))
        .map(|(filament, group)| (*filament, *group))
        .collect();
    let group = |filament: i32| group_of.get(&filament).copied().unwrap_or(-1);
    let preloads = |group: i32| {
        usize::try_from(group)
            .ok()
            .and_then(|group| preload.get(group))
            .copied()
            .unwrap_or(false)
    };

    let mut location: HashMap<i32, Location> =
        filaments.iter().map(|f| (*f, Location::Spool)).collect();
    let mut filament_extruder: HashMap<i32, i32> = HashMap::new();
    let mut extruder_filament: HashMap<i32, i32> = HashMap::new();
    let mut occupied: HashMap<i32, HashSet<i32>> = HashMap::new();
    let at = |location: &mut HashMap<i32, Location>, filament: i32| {
        *location.entry(filament).or_insert(Location::Spool)
    };

    let mut total = 0.0f64;
    for (i, &(next, nozzle)) in changes.iter().enumerate() {
        let Some(&extruder) = extruder_of.get(&nozzle) else {
            continue;
        };
        let held = extruder_filament.get(&extruder).copied().unwrap_or(-1);
        let next_group = group(next);
        let held_group = if held == -1 { -1 } else { group(held) };

        // Whatever else of the next filament's group is out goes back.
        if let Some(out) = occupied.get_mut(&next_group) {
            for other in out
                .iter()
                .copied()
                .filter(|other| *other != next)
                .collect::<Vec<_>>()
            {
                match at(&mut location, other) {
                    Location::Extruder => {
                        total += f64::from(unload_extruder_to_selector + unload_spool_to_selector);
                        if let Some(e) = filament_extruder.remove(&other) {
                            extruder_filament.remove(&e);
                        }
                    }
                    Location::Selector => total += f64::from(unload_spool_to_selector),
                    Location::Spool => {}
                }
                location.insert(other, Location::Spool);
            }
            out.clear();
        }

        // The filament in the extruder comes out, while the next one may
        // come up to the selector.
        let mut unloaded = false;
        let mut unload_time = 0.0f32;
        if held != -1 && held != next && at(&mut location, held) == Location::Extruder {
            if preloads(held_group) && held_group != next_group {
                unload_time = unload_extruder_to_selector;
                location.insert(held, Location::Selector);
            } else {
                unload_time = unload_extruder_to_selector + unload_spool_to_selector;
                location.insert(held, Location::Spool);
                if let Some(out) = occupied.get_mut(&held_group) {
                    out.remove(&held);
                }
            }
            extruder_filament.remove(&extruder);
            filament_extruder.remove(&held);
            unloaded = true;
        }
        let mut preload_time = 0.0f32;
        if unloaded
            && at(&mut location, next) == Location::Spool
            && held_group != next_group
            && preloads(next_group)
        {
            preload_time = load_spool_to_selector;
            location.insert(next, Location::Selector);
            occupied.entry(next_group).or_default().insert(next);
        }
        total += f64::from(unload_time.max(preload_time));

        // The next filament goes in, while the one after may come up.
        let load_time = match at(&mut location, next) {
            Location::Spool => load_spool_to_selector + load_selector_to_extruder,
            Location::Selector => load_selector_to_extruder,
            Location::Extruder => 0.0,
        };
        extruder_filament.insert(extruder, next);
        location.insert(next, Location::Extruder);
        filament_extruder.insert(next, extruder);
        occupied.entry(next_group).or_default().insert(next);

        let mut ahead_time = 0.0f32;
        if let Some(&(after, _)) = changes.get(i + 1) {
            let after_group = group(after);
            if at(&mut location, after) == Location::Spool
                && after_group != next_group
                && preloads(after_group)
                && occupied.entry(after_group).or_default().is_empty()
            {
                ahead_time = load_spool_to_selector;
                location.insert(after, Location::Selector);
                occupied.entry(after_group).or_default().insert(after);
            }
        }
        total += f64::from(load_time.max(ahead_time));
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nozzle::{VolumeType, nozzle_list};

    #[test]
    fn each_change_unloads_the_last_and_loads_the_next() {
        let nozzles = nozzle_list(&[
            ("0.4".into(), VolumeType::STANDARD, 0, 1),
            ("0.4".into(), VolumeType::STANDARD, 1, 1),
        ]);
        let times = ChangeTimes::from_machine(26.0, 26.0);
        // 0 and 2 on the left, 1 on the right: load 0, load 1, swap 0 for
        // 2 (unload and load), then 1 again (already in).
        let changes = [(0, 0), (1, 1), (2, 0), (1, 1)];
        let time = simulate_change_time(&[0, 1, 2], &nozzles, &changes, &[0, 1, 0], &times, &[]);
        assert_eq!(time, 26.0 * 4.0);
        // The same filament again on its own nozzle is free.
        let time = simulate_change_time(&[0], &nozzles, &[(0, 1), (0, 1)], &[1], &times, &[]);
        assert_eq!(time, 26.0);
    }
}
