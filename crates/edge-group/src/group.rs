//! Which nozzle each filament prints from (FilamentGroup.cpp,
//! FilamentGroupUtils.cpp and ToolOrdering.cpp
//! `get_recommended_filament_maps`, `build_filament_group_context`,
//! `prepare_flush_matrices`).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::nozzle::{GroupResult, NozzleInfo, VolumeType};
use crate::order::{LayerSequence, custom_sequence, reorder_for_nozzles};
use crate::time::{ChangeTimes, simulate_change_time};

/// From filament `i` to filament `j` on one nozzle, mm³.
pub type FlushMatrix = Vec<Vec<f32>>;

/// Groupings to try one by one at most (`ENUM_THRESHOLD`); past it the
/// reference searches by k-medoids under a clock, which is not ported.
pub const ENUM_LIMIT: i64 = 10_000;
/// Where counting groupings stops early (`ENUM_EARLY_EXIT`).
const ENUM_EARLY_EXIT: i64 = 10_000_000;
const GOLDEN_RATIO_32: u64 = 0x9e37_79b9;
/// A grouping that leaves the main extruder fewer than half the filaments
/// scores this much worse (`ABSOLUTE_FLUSH_GAP_TOLERANCE`).
const MASTER_SHORT: f64 = 10.0;
/// Flush volume to the score's seconds: density, flush time per gram, a
/// factor of two (`FLUSH_VOLUME_TO_SCORE`), worked as there.
const FLUSH_VOLUME_TO_SCORE: f64 = 1.26 * 180.0 * 2.0 / 1000.0;
const UNPLACEABLE_LIMIT_REWARD: i32 = 10_000;
const MAX_SIZE_LIMIT_REWARD: i32 = 5_000;
const SUPPORT_PREFER_REWARD: i32 = 100;

/// `filament_map_mode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    AutoForFlush,
    AutoForMatch,
    Manual,
    NozzleManual,
    AutoForQuality,
}

impl Mode {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "Auto For Flush" => Self::AutoForFlush,
            "Auto For Match" => Self::AutoForMatch,
            "Manual" => Self::Manual,
            "Nozzle Manual" => Self::NozzleManual,
            "Auto For Quality" => Self::AutoForQuality,
            _ => return None,
        })
    }
}

/// What a filament prints (`FilamentUsageType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Usage {
    SupportOnly,
    ModelOnly,
    Hybrid,
}

/// The printer.
#[derive(Clone, Debug)]
pub struct Machine {
    /// How many extruders (`nozzle_diameter`'s count).
    pub extruders: usize,
    /// Its nozzles, one to an extruder without a rack.
    pub nozzles: Vec<NozzleInfo>,
    /// The main extruder, from 0 (`master_extruder_id` - 1).
    pub master_extruder: i32,
    /// How many filaments each extruder takes before a grouping is the
    /// worse for it (`calc_max_group_size`: the printer's spool count, one
    /// with no printer to ask; every filament with a filament switcher).
    pub max_group_size: Vec<i32>,
    /// Extruders that would rather print support (a Bowden extruder).
    pub prefer_non_model: Vec<bool>,
}

/// What the grouping goes by.
#[derive(Clone, Debug)]
pub struct Input {
    pub mode: Mode,
    /// How many filaments the project has (`filament_colour`'s count).
    pub filament_count: usize,
    /// Each layer's filaments, from 0, in the order the layer lists them
    /// before it is put in order (`LayerTools::extruders` at the time).
    pub layer_filaments: Vec<Vec<u32>>,
    /// One flush matrix to each extruder, multiplier applied
    /// ([`prepare_flush_matrices`]).
    pub flush: Vec<FlushMatrix>,
    /// For each filament what it prints and its diameter (0 unknown).
    pub usage: Vec<Usage>,
    pub diameters: Vec<f32>,
    /// For each extruder, the filaments it cannot print because of where
    /// or how high they print (`geometric_unprintables`).
    pub geometric: Vec<BTreeSet<i32>>,
    /// The plate's own orders for its other layers.
    pub sequences: Vec<LayerSequence>,
    pub times: ChangeTimes,
    /// `filament_map` as the project gives it, extruders from 1.
    pub manual_map: Vec<i32>,
}

/// The answer: which nozzle each filament prints from, on every layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grouping {
    pub result: GroupResult,
    /// Each filament's extruder from 1, as the reference writes
    /// `filament_map` back (`get_extruder_map(false)`).
    pub filament_map: Vec<i32>,
}

/// Each nozzle's flush matrix, `filaments` × `filaments`, times its
/// multiplier (`prepare_flush_matrices`): `values` is
/// `flush_volumes_matrix`, `multiplier` `flush_multiplier`, `fast`
/// `flush_multiplier_fast` where `prime_volume_mode` is "Fast".
///
/// The reference cuts `values` into one equal part per nozzle, whatever
/// its length (PrintConfig.hpp `get_flush_volumes_matrix`), and pads a
/// part shorter than `filaments` × `filaments` with 0. A project that
/// carries one matrix for all nozzles (as this fork's projects for the H2D
/// do) is thereby read as half a matrix on each nozzle, rows out of place,
/// the rest 0: with one filament used, the right nozzle's "matrix" makes
/// every layer cost 639 mm³ and the filament goes to the left nozzle
/// (measured on fx/m_one against fx/h2d_cube). That is not followed:
/// `as_reference` false gives every nozzle the one matrix, and refuses a
/// length that is neither one matrix nor one to each nozzle. With
/// `as_reference` the reference's reading is kept, to check the port.
pub fn prepare_flush_matrices(
    values: &[f64],
    extruders: usize,
    filaments: usize,
    multiplier: f64,
    fast: Option<&[f64]>,
    as_reference: bool,
) -> Result<Vec<FlushMatrix>, String> {
    let square = filaments * filaments;
    let part = |nozzle: usize| -> Vec<f32> {
        let (from, to) = if extruders == 1 {
            (0, values.len())
        } else {
            let each = values.len() / extruders;
            (each * nozzle, each * (nozzle + 1))
        };
        values[from..to].iter().map(|v| *v as f32).collect()
    };
    let mut matrices = Vec::with_capacity(extruders);
    for nozzle in 0..extruders {
        let mut flat = if as_reference || values.len() == extruders * square {
            part(nozzle)
        } else if values.len() == square {
            values.iter().map(|v| *v as f32).collect()
        } else {
            return Err(format!(
                "flush_volumes_matrix has {} values, neither {square} ({filaments} filaments) nor \
                 {} (one matrix to each of {extruders} nozzles): the filament grouping cannot read it",
                values.len(),
                extruders * square
            ));
        };
        if flat.len() < square {
            flat.resize(square, 0.0);
        }
        matrices.push(
            (0..filaments)
                .map(|row| flat[row * filaments..(row + 1) * filaments].to_vec())
                .collect::<FlushMatrix>(),
        );
    }
    let mut multipliers = fast.map(<[f64]>::to_vec).unwrap_or_default();
    multipliers.resize(extruders, multiplier);
    for (matrix, multiplier) in matrices.iter_mut().zip(multipliers) {
        for value in matrix.iter_mut().flatten() {
            *value = (f64::from(*value) * multiplier) as f32;
        }
    }
    Ok(matrices)
}

/// The filaments the plate uses, ascending (`collect_sorted_used_filaments`).
fn used_filaments(layer_filaments: &[Vec<u32>]) -> Vec<u32> {
    let used: BTreeSet<u32> = layer_filaments.iter().flatten().copied().collect();
    used.into_iter().collect()
}

/// Removes what both sets hold from both; whether there was any
/// (FilamentGroupUtils.cpp `remove_intersection`).
fn remove_intersection(a: &mut BTreeSet<i32>, b: &mut BTreeSet<i32>) -> bool {
    let both: Vec<i32> = a.intersection(b).copied().collect();
    for item in &both {
        a.remove(item);
        b.remove(item);
    }
    !both.is_empty()
}

/// For each of two extruders the filaments it cannot print, from what the
/// printer cannot print and where the parts lie
/// (`collect_unprintable_limits`): a filament neither can print is left to
/// both; a filament the two lists put on different extruders goes by the
/// first.
fn collect_unprintable_limits(
    physical: &[BTreeSet<i32>],
    geometric: &[BTreeSet<i32>],
) -> Vec<BTreeSet<i32>> {
    let resized = |sets: &[BTreeSet<i32>]| {
        let mut sets = sets.to_vec();
        sets.resize(2, BTreeSet::new());
        let (a, b) = sets.split_at_mut(1);
        remove_intersection(&mut a[0], &mut b[0]);
        sets
    };
    let mut on: BTreeMap<i32, usize> = BTreeMap::new();
    for sets in [resized(physical), resized(geometric)] {
        for (extruder, set) in sets.iter().enumerate() {
            for filament in set {
                on.entry(*filament).or_insert(extruder);
            }
        }
    }
    let mut limits = vec![BTreeSet::new(); 2];
    for (filament, extruder) in on {
        limits[extruder].insert(filament);
    }
    limits
}

/// For each used filament (by its place in `used`) the extruders it
/// cannot go to, a filament neither can take left free
/// (`extract_unprintable_limit_indices`).
fn unprintable_limit_indices(
    unprintable: &[BTreeSet<i32>],
    used: &[u32],
) -> BTreeMap<usize, Vec<i32>> {
    let mut places: Vec<BTreeSet<i32>> = (unprintable.iter())
        .map(|set| {
            (set.iter())
                .filter_map(|f| used.iter().position(|u| i64::from(*u) == i64::from(*f)))
                .map(|place| place as i32)
                .collect()
        })
        .collect();
    if places.len() > 1 {
        let (a, b) = places.split_at_mut(1);
        remove_intersection(&mut a[0], &mut b[0]);
    }
    let mut limits: BTreeMap<usize, Vec<i32>> = BTreeMap::new();
    for (extruder, set) in places.iter().enumerate() {
        for place in set {
            limits
                .entry(*place as usize)
                .or_default()
                .push(extruder as i32);
        }
    }
    for list in limits.values_mut() {
        list.sort_unstable();
        list.dedup();
    }
    limits
}

/// FilamentGroup.cpp `filament_diameter_matches_nozzle_volume`.
fn diameter_matches(diameter: f32, volume_type: VolumeType, machine_has_tpu285: bool) -> bool {
    if diameter <= 0.0 {
        return true;
    }
    let thick = diameter > 2.5;
    let thick_path = volume_type == VolumeType::TPU_285;
    if thick && !machine_has_tpu285 {
        return true;
    }
    thick == thick_path
}

/// FilamentGroup.cpp `fnv_hash_nozzle`.
fn fnv_hash_nozzle(volume_type: i32, right: bool, loaded: i32) -> u64 {
    const OFFSET: u64 = 14_695_981_039_346_656_037;
    const PRIME: u64 = 1_099_511_628_211;
    let mut h = OFFSET;
    h ^= (volume_type as i64 as u64).wrapping_add(0xA5A5_A5A5_A5A5_A5A5);
    h = h.wrapping_mul(PRIME);
    h ^= u64::from(right).wrapping_add(0x5A5A_5A5A_5A5A_5A5A);
    h = h.wrapping_mul(PRIME);
    if loaded >= 0 {
        h ^= (loaded as u64).wrapping_add(0x3C3C_3C3C_3C3C_3C3C);
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// How many groupings there are to try, those of nozzles alike counted
/// once (`estimate_dedup_enum_count`; no nozzle is loaded here).
fn enum_count(k: usize, n: usize, nozzles: &[NozzleInfo]) -> i64 {
    if n == 0 || k == 0 {
        return 0;
    }
    let mut total: i64 = 1;
    for _ in 0..n {
        total *= k as i64;
        if total > ENUM_EARLY_EXIT {
            return total;
        }
    }
    if k <= 1 {
        return total;
    }
    let mut alike: BTreeMap<u64, i64> = BTreeMap::new();
    for nozzle in nozzles {
        *alike
            .entry(fnv_hash_nozzle(
                nozzle.volume_type.0,
                nozzle.group_id > 0,
                -1,
            ))
            .or_default() += 1;
    }
    let factor: i64 = alike
        .values()
        .map(|count| (2..=*count).product::<i64>())
        .product();
    total / factor.max(1)
}

/// The grouping's working state (`FilamentGroupContext` and
/// `FilamentGroup`), for a printer of one nozzle to an extruder.
struct Context<'a> {
    machine: &'a Machine,
    input: &'a Input,
    unprintable: Vec<BTreeSet<i32>>,
}

impl Context<'_> {
    fn custom(&self) -> impl Fn(usize) -> Option<Vec<i64>> + '_ {
        |layer| custom_sequence(&self.input.sequences, layer).map(<[i64]>::to_vec)
    }

    /// The score of putting the filaments where `map` says: what they
    /// flush, in seconds, and how long their changes take
    /// (`full_evaluate`). The reference adds each filament's print time on
    /// its extruder where `group_algo_with_time` is set, from a table it
    /// never fills (`Print::set_filament_print_time` has no caller), so
    /// that is nothing.
    fn evaluate(&self, used: &[u32], map: &[i32]) -> f64 {
        let nozzles = &self.machine.nozzles;
        let Some(grouping) = GroupResult::create(map.to_vec(), nozzles.clone(), used.to_vec())
        else {
            return 0.0;
        };
        let custom = self.custom();
        let (flush, sequences) = reorder_for_nozzles(
            used,
            &grouping,
            &self.input.layer_filaments,
            &self.input.flush,
            &custom,
        );
        let mut changes: Vec<(i32, i32)> = Vec::new();
        let mut previous = (-1, -1);
        for filament in sequences.iter().flatten() {
            let Some(nozzle) = grouping.nozzle_for(i64::from(*filament)) else {
                continue;
            };
            let change = (*filament as i32, nozzle.group_id);
            if change == previous {
                continue;
            }
            changes.push(change);
            previous = change;
        }
        let mut change_time = 0.0;
        if !sequences.is_empty() {
            let filaments: Vec<i32> = used.iter().map(|f| *f as i32).collect();
            let groups: Vec<i32> = (used.iter())
                .map(|f| {
                    (map.get(*f as usize))
                        .and_then(|nozzle| usize::try_from(*nozzle).ok())
                        .and_then(|nozzle| nozzles.get(nozzle))
                        .map_or(0, |nozzle| nozzle.extruder_id)
                })
                .collect();
            change_time = simulate_change_time(
                &filaments,
                nozzles,
                &changes,
                &groups,
                &self.input.times,
                &[],
            );
        }
        let print_time = 0.0;
        f64::from(flush) * FLUSH_VOLUME_TO_SCORE + (change_time + print_time)
    }

    /// For each used filament (by place) the nozzles it cannot go to
    /// (`rebuild_nozzle_unprintables`; every filament takes any kind of
    /// nozzle in the automatic modes, and no nozzle kind is refused a
    /// filament here).
    fn nozzle_unprintables(&self, used: &[u32]) -> BTreeMap<usize, Vec<i32>> {
        let extruder_limits = unprintable_limit_indices(&self.unprintable, used);
        let nozzles = &self.machine.nozzles;
        let has_tpu285 = nozzles.iter().any(|n| n.volume_type == VolumeType::TPU_285);
        let mut limits = BTreeMap::new();
        for (place, filament) in used.iter().enumerate() {
            let unexpected = extruder_limits.get(&place).cloned().unwrap_or_default();
            let diameter = self.input.diameters.get(*filament as usize).copied();
            let mut refused: Vec<i32> = (nozzles.iter().enumerate())
                .filter(|(_, nozzle)| {
                    unexpected.contains(&nozzle.extruder_id)
                        || diameter
                            .is_some_and(|d| !diameter_matches(d, nozzle.volume_type, has_tpu285))
                })
                .map(|(index, _)| index as i32)
                .collect();
            if refused.is_empty() {
                continue;
            }
            refused.sort_unstable();
            refused.dedup();
            limits.insert(place, refused);
        }
        limits
    }

    /// Tries every way of putting the used filaments on the `k` nozzles
    /// and keeps the best (`calc_group_by_enum`): first by how many of the
    /// rules each meets (every filament on a nozzle it may use, no nozzle
    /// over its share, support on a nozzle that prefers it), then by score;
    /// of two alike the first tried.
    fn group_by_enum(
        &self,
        k: usize,
        used: &[u32],
        unplaceable: &BTreeMap<usize, Vec<i32>>,
    ) -> Vec<i32> {
        let n = used.len();
        let machine = self.machine;
        let master = machine.master_extruder;
        let count = self.input.filament_count;
        let candidates: Vec<Vec<i32>> = (0..n)
            .map(|i| {
                let refused = unplaceable.get(&i);
                (0..k as i32)
                    .filter(|g| !refused.is_some_and(|refused| refused.contains(g)))
                    .collect()
            })
            .collect();
        let mut nozzle_hash = vec![0u64; k];
        for nozzle in &machine.nozzles {
            if let Ok(id) = usize::try_from(nozzle.group_id)
                && id < k
            {
                nozzle_hash[id] = fnv_hash_nozzle(nozzle.volume_type.0, nozzle.group_id > 0, -1);
            }
        }
        let mut seen: HashSet<Vec<u64>> = HashSet::new();
        let mut best_map = vec![master; count];
        let mut best_score = f64::MAX;
        let mut best_prefer = 0;
        let total = (k as f64).powi(n as i32) as i64;
        for mask in 0..total {
            let mut num = mask;
            let mut on_nozzle: HashMap<usize, Vec<usize>> = HashMap::new();
            let mut group_count = vec![0i32; k];
            let mut labels = vec![0usize; n];
            for (i, label) in labels.iter_mut().enumerate() {
                let g = (num % k as i64) as usize;
                num /= k as i64;
                *label = g;
                on_nozzle.entry(g).or_default().push(i);
                group_count[g] += 1;
            }
            // Groupings that put the same filaments on nozzles alike are
            // tried once.
            let mut hashes: Vec<u64> = (on_nozzle.iter())
                .map(|(g, filaments)| {
                    let mask = (filaments.iter())
                        .fold(0u64, |mask, f| mask | 1u64.wrapping_shl(*f as u32));
                    let h = nozzle_hash[*g];
                    h ^ mask
                        .wrapping_add(GOLDEN_RATIO_32)
                        .wrapping_add(h << 6)
                        .wrapping_add(h >> 2)
                })
                .collect();
            hashes.sort_unstable();
            if !seen.insert(hashes) {
                continue;
            }

            let placeable = (0..n)
                .filter(|i| candidates[*i].contains(&(labels[*i] as i32)))
                .count();
            let mut prefer = placeable as i32 * UNPLACEABLE_LIMIT_REWARD;
            let size_ok = (0..k).all(|g| {
                machine
                    .max_group_size
                    .get(g)
                    .is_none_or(|most| group_count[g] <= *most)
            });
            if size_ok {
                prefer += MAX_SIZE_LIMIT_REWARD;
            }
            for g in 0..k {
                if machine.prefer_non_model.get(g) == Some(&true) {
                    for i in on_nozzle.get(&g).into_iter().flatten() {
                        if self.input.usage.get(used[*i] as usize) == Some(&Usage::SupportOnly) {
                            prefer += SUPPORT_PREFER_REWARD;
                        }
                    }
                }
            }

            let mut map = vec![master; count];
            for (i, filament) in used.iter().enumerate() {
                if let Some(slot) = map.get_mut(*filament as usize) {
                    *slot = labels[i] as i32;
                }
            }
            let mut score = self.evaluate(used, &map);
            if let Ok(m) = usize::try_from(master)
                && m < k
                && (group_count[m] as usize) < n.div_ceil(2)
            {
                score += MASTER_SHORT;
            }
            if prefer > best_prefer || (prefer == best_prefer && score < best_score) {
                best_score = score;
                best_prefer = prefer;
                best_map = map;
            }
        }
        best_map
    }
}

/// Which nozzle each of the project's filaments prints from, as the
/// reference decides it for a printer of `machine.extruders` extruders of
/// one nozzle each (`get_recommended_filament_maps`). None where there is
/// nothing to group (no layers), as the reference leaves `filament_map`
/// then as it is.
pub fn group_filaments(machine: &Machine, input: &Input) -> Result<Option<Grouping>, String> {
    if input.layer_filaments.is_empty() {
        return Ok(None);
    }
    let used = used_filaments(&input.layer_filaments);
    let nozzles = machine.nozzles.clone();
    let finish = |map: Vec<i32>| {
        Ok(
            GroupResult::create(map, nozzles.clone(), used.clone()).map(|result| Grouping {
                filament_map: result.extruder_map(false),
                result,
            }),
        )
    };
    if input.mode == Mode::Manual {
        return finish(input.manual_map.iter().map(|e| e - 1).collect());
    }
    let mut map = vec![machine.master_extruder; input.filament_count];
    if machine.extruders > 2 {
        // Each filament keeps its own tool.
        for (f, slot) in map.iter_mut().enumerate() {
            *slot = f.min(machine.extruders - 1) as i32;
        }
        return finish(map);
    }
    if machine.extruders > 1 {
        // "Auto For Match" groups for flush without a printer's spools
        // to match (`calc_filament_group_for_match` finds none and the
        // reference falls back), as every other automatic mode does.
        let physical = vec![BTreeSet::new(); machine.extruders];
        let mut unprintable = collect_unprintable_limits(&physical, &input.geometric);
        for filament in &used {
            let f = *filament as i32;
            if unprintable[0].contains(&f) && unprintable[1].contains(&f) {
                unprintable[0].remove(&f);
                unprintable[1].remove(&f);
            }
        }
        let context = Context {
            machine,
            input,
            unprintable,
        };
        let k = machine.nozzles.len();
        let unplaceable = context.nozzle_unprintables(&used);
        let tries = enum_count(k, used.len(), &machine.nozzles);
        if tries >= ENUM_LIMIT {
            return Err(format!(
                "{} filaments on {k} nozzles: the reference groups that many by a search \
                 bounded by a three-second clock (k-medoids), whose answer depends on the \
                 machine; not done",
                used.len()
            ));
        }
        map = context.group_by_enum(k, &used, &unplaceable);
    }
    finish(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nozzle::nozzle_list;

    /// Two nozzles of one kind, one to each extruder, the H2D's shape.
    fn h2d() -> Machine {
        Machine {
            extruders: 2,
            nozzles: nozzle_list(&[
                ("0.4".into(), VolumeType::STANDARD, 0, 1),
                ("0.4".into(), VolumeType::STANDARD, 1, 1),
            ]),
            master_extruder: 1,
            max_group_size: vec![1, 1],
            prefer_non_model: vec![false, false],
        }
    }

    /// `make_context` of the reference's test_dual_nozzle_group.cpp: every
    /// change 280 mm³ on both nozzles, one layer printing every filament.
    fn input(filaments: usize) -> Input {
        let matrix: FlushMatrix = (0..filaments)
            .map(|i| {
                (0..filaments)
                    .map(|j| if i == j { 0.0 } else { 280.0 })
                    .collect()
            })
            .collect();
        Input {
            mode: Mode::AutoForFlush,
            filament_count: filaments,
            layer_filaments: vec![(0..filaments as u32).collect()],
            flush: vec![matrix.clone(), matrix],
            usage: vec![Usage::ModelOnly; filaments],
            diameters: vec![0.0; filaments],
            geometric: vec![BTreeSet::new(); 2],
            sequences: Vec::new(),
            times: ChangeTimes::default(),
            manual_map: vec![1],
        }
    }

    #[test]
    fn two_filaments_go_one_to_each_nozzle() {
        // test_dual_nozzle_group.cpp "Two filaments group onto a
        // dual-nozzle machine's two nozzles".
        let grouping = group_filaments(&h2d(), &input(2)).unwrap().unwrap();
        let map = grouping.result.map;
        assert_eq!(map.len(), 2);
        assert!(map.iter().all(|nozzle| (0..2).contains(nozzle)));
        assert_ne!(map[0], map[1]);
    }

    #[test]
    fn four_filaments_stay_on_the_two_nozzles() {
        // "Four filaments group onto a dual-nozzle machine without leaving
        // the matrices".
        let mut machine = h2d();
        machine.max_group_size = vec![4, 4];
        let grouping = group_filaments(&machine, &input(4)).unwrap().unwrap();
        assert_eq!(grouping.result.map.len(), 4);
        assert!(
            grouping
                .result
                .map
                .iter()
                .all(|nozzle| (0..2).contains(nozzle))
        );
        // The same input, the same answer.
        assert_eq!(
            group_filaments(&machine, &input(4)).unwrap().unwrap(),
            grouping
        );
    }

    #[test]
    fn sixteen_filaments_are_refused() {
        // The reference's k-medoids search (its third test case) is not
        // ported: past the limit the grouping says so.
        let mut machine = h2d();
        machine.max_group_size = vec![16, 16];
        let said = group_filaments(&machine, &input(16)).unwrap_err();
        assert!(said.contains("k-medoids"), "{said}");
        assert_eq!(enum_count(2, 13, &machine.nozzles), 8192);
        assert_eq!(enum_count(2, 14, &machine.nozzles), 16384);
    }

    #[test]
    fn one_filament_goes_to_the_main_extruder() {
        // Alone it costs the same on either nozzle; the left nozzle, not
        // the main one, scores ten worse.
        let mut one = input(1);
        one.times = ChangeTimes::from_machine(26.0, 26.0);
        let grouping = group_filaments(&h2d(), &one).unwrap().unwrap();
        assert_eq!(grouping.filament_map, [2]);
    }

    #[test]
    fn manual_maps_are_taken_as_given() {
        let mut manual = input(3);
        manual.mode = Mode::Manual;
        manual.manual_map = vec![2, 1, 3];
        let grouping = group_filaments(&h2d(), &manual).unwrap().unwrap();
        // An extruder past the printer's reads as none (0).
        assert_eq!(grouping.filament_map, [2, 1, 0]);
    }

    #[test]
    fn a_filament_one_nozzle_cannot_print_goes_to_the_other() {
        let mut machine = h2d();
        machine.max_group_size = vec![4, 4];
        let mut three = input(3);
        three.geometric = vec![BTreeSet::from([0, 1, 2]), BTreeSet::new()];
        let grouping = group_filaments(&machine, &three).unwrap().unwrap();
        assert_eq!(grouping.filament_map, [2, 2, 2]);
    }

    #[test]
    fn flush_matrices_are_cut_per_nozzle() {
        // One matrix to each nozzle.
        let values = [0.0, 1.0, 2.0, 0.0, 0.0, 3.0, 4.0, 0.0];
        let matrices = prepare_flush_matrices(&values, 2, 2, 1.0, None, false).unwrap();
        assert_eq!(matrices[1], [vec![0.0, 3.0], vec![4.0, 0.0]]);
        // One for all: each nozzle has it, where the reference cuts it in
        // halves and pads them with 0.
        let one = [0.0, 199.0, 639.0, 0.0];
        let matrices = prepare_flush_matrices(&one, 2, 2, 0.5, None, false).unwrap();
        assert_eq!(matrices[1], [vec![0.0, 99.5], vec![319.5, 0.0]]);
        let halves = prepare_flush_matrices(&one, 2, 2, 1.0, None, true).unwrap();
        assert_eq!(halves[0], [vec![0.0, 199.0], vec![0.0, 0.0]]);
        assert_eq!(halves[1], [vec![639.0, 0.0], vec![0.0, 0.0]]);
        // The fast multiplier for the nozzles it names, the plain one after.
        let fast = prepare_flush_matrices(&one, 2, 2, 1.0, Some(&[2.0]), false).unwrap();
        assert_eq!((fast[0][0][1], fast[1][0][1]), (398.0, 199.0));
        assert!(prepare_flush_matrices(&one[..3], 2, 2, 1.0, None, false).is_err());
    }
}
