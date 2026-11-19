//! The order filaments are taken in on each layer, nozzle by nozzle, and
//! what it flushes (GCode/ToolOrderUtils.cpp `solve_extruder_order`,
//! `get_extruders_order`, `reorder_filaments_for_minimum_flush_volume_base`,
//! `reorder_filaments_for_multi_nozzle_extruder`). The grouping scores a
//! way of putting filaments on nozzles by it.
//!
//! The reference adds a float to an int total, which rounds the float sum
//! towards zero after each addition; that is kept (`add`), since the totals
//! decide the grouping and ties between groupings.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::nozzle::GroupResult;

/// A plate's own order for a range of layers (ParameterUtils.cpp
/// `LayerPrintSequence`): its first and last layer, counting from 1, and
/// the filaments in order, counting from 1.
pub type LayerSequence = ((i64, i64), Vec<i64>);

/// The plate's own orders for its other layers (ParameterUtils.cpp
/// `get_other_layers_print_sequence`): `values` in `ranges` groups, each
/// its first and last layer and then the filaments. None where the count
/// does not divide them into groups of three or more.
pub fn other_layers_sequences(ranges: i64, values: &[i64]) -> Vec<LayerSequence> {
    let Ok(ranges) = usize::try_from(ranges) else {
        return Vec::new();
    };
    if ranges == 0
        || values.is_empty()
        || !values.len().is_multiple_of(ranges)
        || values.len() / ranges < 3
    {
        return Vec::new();
    }
    (values.chunks(values.len() / ranges))
        .map(|group| ((group[0], group[1]), group[2..].to_vec()))
        .collect()
}

/// The order the plate gives layer `layer` (from 0), as filament numbers
/// from 1: the last range that holds it (ToolOrdering.cpp
/// `create_custom_seq_function` without the first layer's).
pub(crate) fn custom_sequence(sequences: &[LayerSequence], layer: usize) -> Option<&[i64]> {
    let number = layer as i64 + 1;
    (sequences.iter().rev())
        .find(|((first, last), _)| number >= *first && number <= *last)
        .map(|(_, sequence)| sequence.as_slice())
}

/// `total += value` with `total` an int and `value` a float, as C++ does it.
fn add(total: i32, value: f32) -> i32 {
    (total as f32 + value) as i32
}

/// The shortest way through all of `tools` starting from `start`, when
/// going from `a` to `b` costs `cost(a, b)`, and what it costs: the
/// reference's `solve_extruder_order`, kept step for step so that ties
/// fall as its do. `start` need not be among the tools; it is then where
/// the way begins and is left out of the answer. The cost is none where
/// the reference leaves it unset.
pub fn solve_order(
    cost: impl Fn(usize, usize) -> f32,
    tools: &[usize],
    start: Option<usize>,
) -> (Vec<usize>, Option<f32>) {
    let mut all = tools.to_vec();
    let mut added = false;
    let start = match start {
        Some(start) => {
            match all.iter().position(|tool| *tool == start) {
                Some(at) => all.swap(0, at),
                None => {
                    all.insert(0, start);
                    added = true;
                }
            }
            start
        }
        None => match all.first() {
            Some(first) => *first,
            None => return (Vec::new(), None),
        },
    };
    let count = all.len();
    let states = 1usize << count;
    // The least cost of having been to every tool in `state` and standing
    // at `target`, and which tool that way came from.
    let mut least = vec![vec![0x7fff_ffff as f32; count]; states];
    let mut from = vec![vec![usize::MAX; count]; states];
    least[1][0] = 0.0;
    for state in 0..states {
        if state & 1 == 0 {
            continue;
        }
        for target in 0..count {
            if state >> target & 1 == 0 {
                continue;
            }
            for middle in 0..count {
                if state >> middle & 1 == 0 {
                    continue;
                }
                let so_far = least[state - (1 << target)][middle] + cost(all[middle], all[target]);
                if least[state][target] > so_far {
                    least[state][target] = so_far;
                    from[state][target] = middle;
                }
            }
        }
    }
    // The first tool, other than the start, that the cheapest way ends at.
    let whole = states - 1;
    let (mut best, mut end, mut found) = (f32::MAX, 0, None);
    for last in 0..count {
        if all[last] != start && best > least[whole][last] {
            best = least[whole][last];
            end = last;
            found = Some(best);
        }
    }
    let mut path = Vec::with_capacity(count);
    let (mut state, mut at) = (whole, end);
    while at != usize::MAX {
        path.push(all[at]);
        let before = from[state][at];
        state -= 1 << at;
        at = before;
    }
    if added {
        path.pop();
    }
    path.reverse();
    (path, found)
}

/// The order of `tools` on one nozzle from `start`, and what it flushes by
/// `matrix` (`get_extruders_order` without the look one layer ahead, which
/// the grouping does not ask for).
pub fn extruders_order(
    matrix: &[Vec<f32>],
    tools: &[usize],
    start: Option<usize>,
) -> (Vec<usize>, f32) {
    let at = |a: usize, b: usize| matrix.get(a).and_then(|row| row.get(b)).copied();
    match tools.len() {
        0 => (Vec::new(), 0.0),
        1 => {
            let cost = start.map_or(0.0, |start| at(start, tools[0]).unwrap_or(0.0));
            (tools.to_vec(), cost)
        }
        2..=20 => {
            // A filament past the matrix is left where it is (the guard in
            // `solve_extruder_order`), the cost unset.
            if tools.iter().any(|tool| *tool >= matrix.len()) {
                return (tools.to_vec(), 0.0);
            }
            let (path, cost) = solve_order(|a, b| matrix[a][b], tools, start);
            (path, cost.unwrap_or(0.0))
        }
        _ => greedy(matrix, tools, start),
    }
}

/// The cheapest next filament each time (`solve_extruder_order_with_greedy`),
/// for more than twenty on a nozzle. Its running cost is an int.
fn greedy(matrix: &[Vec<f32>], tools: &[usize], start: Option<usize>) -> (Vec<usize>, f32) {
    let mut cost = 0.0f32;
    let mut order = Vec::with_capacity(tools.len());
    let mut visited = vec![false; tools.len()];
    let mut previous = start;
    for _ in 0..tools.len() {
        let from = match previous {
            Some(from) => from,
            None => tools[visited.iter().position(|seen| !seen).unwrap_or(0)],
        };
        let mut target: Option<usize> = None;
        let mut target_cost = i32::MAX;
        for (k, tool) in tools.iter().enumerate() {
            if visited[k] {
                continue;
            }
            let volume = matrix[from][*tool];
            if volume < target_cost as f32 || (volume == target_cost as f32 && from == *tool) {
                target = Some(k);
                target_cost = volume as i32;
            }
        }
        let Some(k) = target else { break };
        cost += target_cost as f32;
        order.push(tools[k]);
        previous = Some(tools[k]);
        visited[k] = true;
    }
    (order, cost)
}

/// The custom orders of the layers that have one, filaments from 0: each
/// layer's own filaments in the order the plate gives (`custom_layer_sequence_map`).
fn custom_layers(
    layer_filaments: &[Vec<u32>],
    custom: &dyn Fn(usize) -> Option<Vec<i64>>,
) -> BTreeMap<usize, Vec<u32>> {
    let mut map = BTreeMap::new();
    for (layer, filaments) in layer_filaments.iter().enumerate() {
        let Some(sequence) = custom(layer).filter(|sequence| !sequence.is_empty()) else {
            continue;
        };
        let kept = (sequence.iter())
            .filter_map(|number| u32::try_from(number - 1).ok())
            .filter(|filament| filaments.contains(filament))
            .collect();
        map.insert(layer, kept);
    }
    map
}

/// One nozzle's order on every layer, the filaments of `group` only, from
/// `initial`, and what it flushes by `matrix`
/// (`reorder_filaments_for_minimum_flush_volume_base`).
fn reorder_base(
    group: &[u32],
    layer_filaments: &[Vec<u32>],
    matrix: &[Vec<f32>],
    custom: &dyn Fn(usize) -> Option<Vec<i64>>,
    initial: Option<u32>,
) -> (i32, Vec<Vec<u32>>) {
    let custom_layers = custom_layers(layer_filaments, custom);
    let in_group: BTreeSet<u32> = group.iter().copied().collect();
    let mut cost = 0i32;
    let mut sequences = Vec::with_capacity(layer_filaments.len());
    let mut caches: HashMap<u128, (f32, Vec<usize>)> = HashMap::new();
    let mut current = initial.filter(|filament| (*filament as usize) < matrix.len());
    for (layer, filaments) in layer_filaments.iter().enumerate() {
        if let Some(sequence) = custom_layers.get(&layer) {
            let kept: Vec<u32> = (sequence.iter())
                .copied()
                .filter(|filament| in_group.contains(filament))
                .collect();
            let mut previous = current;
            for filament in &kept {
                if let Some(previous) = previous {
                    cost = add(cost, matrix[previous as usize][*filament as usize]);
                }
                previous = Some(*filament);
            }
            if let Some(last) = kept.last() {
                current = Some(*last);
            }
            sequences.push(kept);
            continue;
        }
        let used: Vec<usize> = (filaments.iter())
            .filter(|filament| in_group.contains(filament))
            .map(|filament| *filament as usize)
            .collect();
        // Bits 0-31 the layer's filaments, 64 up the filament in hand.
        let bit = |at: usize| {
            u32::try_from(at)
                .ok()
                .and_then(|at| 1u128.checked_shl(at))
                .unwrap_or(0)
        };
        let mut key = current.map_or(0, |current| bit(64 + current as usize));
        for filament in &used {
            key |= bit(*filament);
        }
        let (layer_cost, sequence) = caches
            .entry(key)
            .or_insert_with(|| {
                let (sequence, cost) =
                    extruders_order(matrix, &used, current.map(|current| current as usize));
                (cost, sequence)
            })
            .clone();
        if let Some(last) = sequence.last() {
            current = Some(*last as u32);
        }
        sequences.push(
            sequence
                .into_iter()
                .map(|filament| filament as u32)
                .collect(),
        );
        cost = add(cost, layer_cost);
    }
    (cost, sequences)
}

/// What `filaments` flush on every layer when each prints from the nozzle
/// `grouping` gives it, nozzle by nozzle with the flush matrix of the
/// nozzle's extruder, and the order they are taken in on each layer: one
/// extruder's nozzles after the other's, starting with the extruder the
/// layer before ended on (`reorder_filaments_for_multi_nozzle_extruder`,
/// with nothing loaded at the start).
pub fn reorder_for_nozzles(
    filaments: &[u32],
    grouping: &GroupResult,
    layer_filaments: &[Vec<u32>],
    flush: &[Vec<Vec<f32>>],
    custom: &dyn Fn(usize) -> Option<Vec<i64>>,
) -> (i32, Vec<Vec<u32>>) {
    let mut nozzle_filaments: BTreeMap<i32, BTreeSet<u32>> = BTreeMap::new();
    let mut extruder_nozzles: BTreeMap<i32, BTreeSet<i32>> = BTreeMap::new();
    for filament in filaments {
        let Some(nozzle) = grouping.nozzle_for(i64::from(*filament)) else {
            continue;
        };
        nozzle_filaments
            .entry(nozzle.group_id)
            .or_default()
            .insert(*filament);
        (extruder_nozzles.entry(nozzle.extruder_id).or_default()).insert(nozzle.group_id);
    }
    let custom_layers = custom_layers(layer_filaments, custom);

    let mut cost = 0i32;
    let mut nozzle_sequences: BTreeMap<i32, Vec<Vec<u32>>> = BTreeMap::new();
    for (nozzle, group) in &nozzle_filaments {
        let extruder = (extruder_nozzles.iter())
            .find(|(_, nozzles)| nozzles.contains(nozzle))
            .map_or(0, |(extruder, _)| *extruder);
        let group: Vec<u32> = group.iter().copied().collect();
        let matrix = flush.get(extruder as usize).map_or(&[][..], Vec::as_slice);
        let (spent, sequences) = reorder_base(&group, layer_filaments, matrix, custom, None);
        cost += spent;
        nozzle_sequences.insert(*nozzle, sequences);
    }

    let extruders: Vec<i32> = extruder_nozzles.keys().copied().collect();
    let nozzles_of: BTreeMap<i32, Vec<i32>> = (extruder_nozzles.iter())
        .map(|(extruder, nozzles)| (*extruder, nozzles.iter().copied().collect()))
        .collect();
    let extruder_of = |filament: u32| {
        grouping
            .nozzle_for(i64::from(filament))
            .map_or(-1, |n| n.extruder_id)
    };
    let nozzle_place = |filament: u32| -> Option<(usize, i32)> {
        let nozzle = grouping.nozzle_for(i64::from(filament))?;
        let list = nozzles_of.get(&nozzle.extruder_id)?;
        let place = (list.iter().position(|id| *id == nozzle.group_id)).unwrap_or(list.len());
        Some((usize::try_from(nozzle.extruder_id).ok()?, place as i32))
    };
    let mut sequences = vec![Vec::new(); layer_filaments.len()];
    let Some(highest) = extruders.iter().max() else {
        return (cost, sequences);
    };
    let mut last_extruder: i32 = 0;
    let mut last_nozzle = vec![0i32; *highest as usize + 1];
    for (layer, out) in sequences.iter_mut().enumerate() {
        if let Some(sequence) = custom_layers.get(&layer) {
            out.clone_from(sequence);
            if let Some(last) = out.last() {
                last_extruder = extruder_of(*last);
                for filament in out.iter() {
                    if let Some((extruder, place)) = nozzle_place(*filament) {
                        last_nozzle[extruder] = place;
                    }
                }
            }
            continue;
        }
        if last_extruder == -1 {
            last_extruder = 0;
        }
        let mut layer_last_extruder = last_extruder;
        let mut layer_last_nozzle = last_nozzle.clone();
        for i in 0..extruders.len() {
            let extruder = extruders[(last_extruder as usize + i) % extruders.len()];
            let nozzles = &nozzles_of[&extruder];
            let e = extruder as usize;
            if last_nozzle[e] == -1 {
                last_nozzle[e] = 0;
            }
            let mut any = false;
            for j in 0..nozzles.len() {
                let place = (last_nozzle[e] as usize + j) % nozzles.len();
                let fragment = &nozzle_sequences[&nozzles[place]][layer];
                if fragment.is_empty() {
                    continue;
                }
                any = true;
                layer_last_nozzle[e] = place as i32;
                out.extend_from_slice(fragment);
            }
            if any {
                layer_last_extruder = extruder;
            }
        }
        last_extruder = layer_last_extruder;
        last_nozzle = layer_last_nozzle;
    }
    (cost, sequences)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nozzle::{VolumeType, nozzle_list};

    fn same(_: usize, _: usize) -> f32 {
        45.0
    }

    #[test]
    fn the_solver_breaks_ties_as_the_reference_s_does() {
        assert_eq!(
            solve_order(same, &[1, 2, 3, 4, 5], Some(2)).0,
            [2, 5, 4, 3, 1]
        );
        assert_eq!(solve_order(same, &[1, 2, 3], None).0, [1, 3, 2]);
        assert_eq!(
            solve_order(same, &[1, 2, 3], Some(7)),
            (vec![3, 2, 1], Some(135.0))
        );
        assert_eq!(solve_order(same, &[], None), (Vec::new(), None));
    }

    #[test]
    fn one_filament_costs_what_coming_to_it_costs() {
        let matrix = vec![vec![0.0, 10.0], vec![20.0, 5.0]];
        assert_eq!(extruders_order(&matrix, &[1], Some(0)), (vec![1], 10.0));
        assert_eq!(extruders_order(&matrix, &[1], Some(1)), (vec![1], 5.0));
        assert_eq!(extruders_order(&matrix, &[1], None), (vec![1], 0.0));
        assert_eq!(extruders_order(&matrix, &[0, 1], None), (vec![0, 1], 10.0));
        // A filament past the matrix leaves the layer as it is, at no cost.
        assert_eq!(extruders_order(&matrix, &[2, 0], None), (vec![2, 0], 0.0));
    }

    #[test]
    fn an_int_total_drops_the_fraction_each_time() {
        assert_eq!(add(add(0, 2.7), 2.7), 4);
    }

    #[test]
    fn plate_orders_are_read_in_ranges() {
        let sequences = other_layers_sequences(2, &[2, 4, 2, 1, 5, 9, 1, 2]);
        assert_eq!(sequences, [((2, 4), vec![2, 1]), ((5, 9), vec![1, 2])]);
        assert_eq!(custom_sequence(&sequences, 0), None);
        assert_eq!(custom_sequence(&sequences, 1), Some(&[2, 1][..]));
        assert_eq!(custom_sequence(&sequences, 4), Some(&[1, 2][..]));
        assert!(other_layers_sequences(2, &[1, 2, 3, 4, 5]).is_empty());
    }

    #[test]
    fn nozzles_flush_apart_and_take_turns() {
        // Filaments 0 and 2 on the left nozzle, 1 on the right; every
        // layer prints all three.
        let nozzles = nozzle_list(&[
            ("0.4".into(), VolumeType::STANDARD, 0, 1),
            ("0.4".into(), VolumeType::STANDARD, 1, 1),
        ]);
        let grouping = GroupResult::create(vec![0, 1, 0], nozzles, vec![0, 1, 2]).unwrap();
        let left = vec![vec![0.0, 0.0, 100.0], vec![0.0; 3], vec![60.5, 0.0, 0.0]];
        let right = vec![vec![0.0; 3]; 3];
        let layers = vec![vec![0, 1, 2]; 3];
        let (cost, sequences) =
            reorder_for_nozzles(&[0, 1, 2], &grouping, &layers, &[left, right], &|_| None);
        // Left: 0 then 2 (100) on the first layer, then from 2: 2, 0 (60)
        // and from 0: 0, 2 (100): 260, the half dropped.
        assert_eq!(cost, 260);
        assert_eq!(sequences, [vec![0, 2, 1], vec![1, 2, 0], vec![0, 2, 1]]);
    }
}
