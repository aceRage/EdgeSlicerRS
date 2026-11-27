//! The order filaments are taken in on each layer, and what it flushes
//! (GCode/ToolOrderUtils.cpp `solve_extruder_order`, `get_extruders_order`,
//! `solve_extruder_order_with_forcast`,
//! `reorder_filaments_for_minimum_flush_volume_base`,
//! `reorder_filaments_for_minimum_flush_volume`,
//! `reorder_filaments_for_multi_nozzle_extruder`).
//!
//! Two routines, as Bambu Studio 02.09.01.70 uses them (the lead's reading,
//! work/cppfix/order/ORDER_RULE.md):
//!
//! - [`reorder_for_nozzles`] (l.1553, nozzle by nozzle, no look-ahead:
//!   `reorder_filaments_for_minimum_flush_volume_base`, l.1019, has
//!   `use_forcast` false): the grouping scores every map with it
//!   (FilamentGroup.cpp l.157), on every printer, and a plate with a nozzle
//!   rack (the H2C) is printed in the order it gives;
//! - [`reorder_for_extruders`] (l.1127, extruder by extruder, looking one
//!   layer ahead where an extruder prints five filaments or fewer on this
//!   layer and the next): a two-extruder plate without a rack (the H2D,
//!   X2D) is printed in the order it gives. In this fork's C++ source its
//!   only caller is the `DEBUG_MULTI_NOZZLE_MCMF` overload (l.1406); Bambu
//!   Studio's own ToolOrdering calls it on the H2D, and C++ fix PR
//!   aceRage/EdgeSlicer#1169 does the same.
//!
//! So on the H2D a map is scored by one routine and printed by the other,
//! on purpose, as Bambu Studio does; on the H2C the two are one.
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
/// [`reorder_for_extruders`] asks for itself).
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

/// The next permutation in lexicographic order (`std::next_permutation`):
/// false, with `items` back in ascending order, after the last.
fn next_permutation(items: &mut [u32]) -> bool {
    let Some(pivot) = (1..items.len()).rev().find(|at| items[at - 1] < items[*at]) else {
        items.reverse();
        return false;
    };
    let pivot = pivot - 1;
    let swap = (pivot + 1..items.len())
        .rev()
        .find(|at| items[*at] > items[pivot])
        .unwrap_or(pivot);
    items.swap(pivot, swap);
    items[pivot + 1..].reverse();
    true
}

/// The order of `current` on one nozzle from `start` that makes this layer
/// and the next (`next`, ordered at its best) cheapest together, fewer
/// filament changes breaking a tie, and what this layer's order flushes
/// (ToolOrderUtils.cpp l.826 `solve_extruder_order_with_forcast`): every
/// permutation of each, in ascending lexicographic order, the first best
/// kept. Sums in single precision, as there.
fn order_looking_ahead(
    matrix: &[Vec<f32>],
    current: &[u32],
    next: &[u32],
    start: Option<u32>,
) -> (Vec<u32>, f32) {
    let w = |a: u32, b: u32| matrix[a as usize][b as usize];
    let changes = |this: &[u32], then: &[u32]| {
        let mut count = 0;
        let mut previous = start;
        for filament in this.iter().chain(then) {
            if previous.is_some_and(|p| p != *filament) {
                count += 1;
            }
            previous = Some(*filament);
        }
        count
    };
    let mut current = current.to_vec();
    current.sort_unstable();
    let mut next = next.to_vec();
    let (mut best_cost, mut best_change, mut best) = (f32::MAX, i32::MAX, Vec::new());
    loop {
        let mut previous = start;
        let mut layer_cost = 0.0f32;
        for filament in &current {
            if let Some(p) = previous {
                layer_cost += w(p, *filament);
            }
            previous = Some(*filament);
        }
        if layer_cost <= best_cost {
            next.sort_unstable();
            loop {
                let mut then = previous;
                let mut total = layer_cost;
                let change = changes(&current, &next);
                for filament in &next {
                    if let Some(p) = then {
                        total += w(p, *filament);
                    }
                    then = Some(*filament);
                }
                if total < best_cost || (total == best_cost && change < best_change) {
                    best_cost = total;
                    best.clone_from(&current);
                    best_change = change;
                }
                if !next_permutation(&mut next) {
                    break;
                }
            }
        }
        if !next_permutation(&mut current) {
            break;
        }
    }
    let mut cost = 0.0f32;
    let mut previous = start;
    for filament in &best {
        if let Some(p) = previous {
            cost += w(p, *filament);
        }
        previous = Some(*filament);
    }
    (best, cost)
}

/// Each extruder's order on every layer and what the layers flush, the
/// filaments of each extruder ordered by that extruder's matrix from the
/// filament it last held, the extruder printed last going first on the
/// next layer (ToolOrderUtils.cpp l.1127-1306
/// `reorder_filaments_for_minimum_flush_volume`, with nothing loaded at
/// the start: an empty `nozzle_status`). `extruder_of[i]` is filament
/// `filaments[i]`'s extruder (0 or 1; any other value is left out, as
/// there). Where a layer and the next each print five filaments or fewer
/// on the extruder, the order looks one layer ahead
/// ([`order_looking_ahead`]); otherwise it is [`extruders_order`]'s.
/// Each extruder keeps one cache, by the layer's filaments, the next
/// layer's where it looks ahead, and the filament in hand.
pub fn reorder_for_extruders(
    filaments: &[u32],
    extruder_of: &[i32],
    layer_filaments: &[Vec<u32>],
    flush: &[Vec<Vec<f32>>],
    custom: &dyn Fn(usize) -> Option<Vec<i64>>,
) -> (i32, Vec<Vec<u32>>) {
    const MAX_N_WITH_FORECAST: usize = 5;
    let mut groups: [BTreeSet<u32>; 2] = [BTreeSet::new(), BTreeSet::new()];
    for (filament, extruder) in filaments.iter().zip(extruder_of) {
        if let Ok(extruder) = usize::try_from(*extruder)
            && extruder < 2
        {
            groups[extruder].insert(*filament);
        }
    }
    let custom_layers = custom_layers(layer_filaments, custom);
    let bit = |at: usize| {
        u32::try_from(at)
            .ok()
            .and_then(|at| 1u128.checked_shl(at))
            .unwrap_or(0)
    };
    let mut cost = 0i32;
    let mut sequences_of: [Vec<Vec<u32>>; 2] = [Vec::new(), Vec::new()];
    for (extruder, group) in groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }
        let matrix = flush.get(extruder).map_or(&[][..], Vec::as_slice);
        let mut current: Option<u32> = None;
        let mut caches: HashMap<u128, (f32, Vec<u32>)> = HashMap::new();
        let in_group = |list: &[u32]| -> Vec<u32> {
            list.iter().copied().filter(|f| group.contains(f)).collect()
        };
        for (layer, filaments) in layer_filaments.iter().enumerate() {
            if let Some(sequence) = custom_layers.get(&layer) {
                let kept = in_group(sequence);
                let mut layer_cost = 0.0f32;
                let mut previous = current;
                for filament in &kept {
                    if let Some(p) = previous {
                        layer_cost += matrix[p as usize][*filament as usize];
                    }
                    previous = Some(*filament);
                }
                cost = add(cost, layer_cost);
                if let Some(last) = kept.last() {
                    current = Some(*last);
                }
                sequences_of[extruder].push(Vec::new());
                continue;
            }
            let used = in_group(filaments);
            let next =
                (layer_filaments.get(layer + 1)).map_or_else(Vec::new, |next| in_group(next));
            let forecast = used.len() <= MAX_N_WITH_FORECAST && next.len() <= MAX_N_WITH_FORECAST;
            let mut key = current.map_or(0, |c| bit(64 + c as usize));
            if forecast {
                for filament in &next {
                    key |= bit(32 + *filament as usize);
                }
            }
            for filament in &used {
                key |= bit(*filament as usize);
            }
            let (layer_cost, sequence) = caches
                .entry(key)
                .or_insert_with(|| match used.len() {
                    0 | 1 => {
                        let as_tools: Vec<usize> = used.iter().map(|f| *f as usize).collect();
                        let (order, cost) =
                            extruders_order(matrix, &as_tools, current.map(|c| c as usize));
                        (cost, order.into_iter().map(|f| f as u32).collect())
                    }
                    _ if forecast => {
                        let (order, cost) = order_looking_ahead(matrix, &used, &next, current);
                        (cost, order)
                    }
                    _ => {
                        let as_tools: Vec<usize> = used.iter().map(|f| *f as usize).collect();
                        let (order, cost) =
                            extruders_order(matrix, &as_tools, current.map(|c| c as usize));
                        (cost, order.into_iter().map(|f| f as u32).collect())
                    }
                })
                .clone();
            if let Some(last) = sequence.last() {
                current = Some(*last);
            }
            sequences_of[extruder].push(sequence);
            cost = add(cost, layer_cost);
        }
    }
    // The extruder that goes first: 0, or after a custom layer the one
    // its last filament is on; a first custom layer at an odd layer
    // turns the start (as there).
    let mut last: usize = 0;
    if let Some((first_layer, sequence)) = custom_layers.iter().next()
        && let Some(first) = sequence.first()
    {
        let first_group = usize::from(!groups[0].contains(first));
        last = if first_layer & 1 == 1 {
            1 - first_group
        } else {
            first_group
        };
    }
    let mut sequences = vec![Vec::new(); layer_filaments.len()];
    for (layer, out) in sequences.iter_mut().enumerate() {
        if let Some(sequence) = custom_layers.get(&layer) {
            out.clone_from(sequence);
            if let Some(back) = out.last() {
                last = usize::from(!groups[0].contains(back));
            }
            continue;
        }
        let part = |extruder: usize| {
            (sequences_of[extruder].get(layer))
                .filter(|part| !part.is_empty())
                .cloned()
        };
        let other = 1 - last;
        if let Some(first) = part(last) {
            out.extend(first);
        }
        if let Some(then) = part(other) {
            out.extend(then);
            last = other;
        }
    }
    (cost, sequences)
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
/// ToolOrderUtils.cpp l.1553-1710, with nothing loaded at the start: an
/// empty `NozzleStatusRecorder`, as both the grouping and the layer order
/// of a rack plate call it). `custom` gives a layer's fixed order, filaments from 1: the
/// layer keeps it, and the nozzles go on from it. More than twenty
/// filaments on a nozzle are ordered greedily (`extruders_order`).
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

    fn two_nozzles() -> Vec<crate::nozzle::NozzleInfo> {
        nozzle_list(&[
            ("0.4".into(), VolumeType::STANDARD, 0, 1),
            ("0.4".into(), VolumeType::STANDARD, 1, 1),
        ])
    }

    /// Each nozzle flushing from what it last held, by its own matrix:
    /// what the printer flushes for `order` after `held`.
    fn rule_flush(order: &[u32], map: &[i32], flush: &[Vec<Vec<f32>>], held: [i64; 2]) -> f32 {
        let mut held = held;
        let mut total = 0.0;
        for filament in order {
            let e = map[*filament as usize] as usize;
            if held[e] >= 0 && held[e] != i64::from(*filament) {
                total += flush[e][held[e] as usize][*filament as usize];
            }
            held[e] = i64::from(*filament);
        }
        total
    }

    /// The C++ fix's own case (aceRage/EdgeSlicer#1169,
    /// tests/libslic3r/test_flush_matrix_reading.cpp "The layer order reads
    /// the matrix of the nozzle each change happens on"): filaments 0 and 1
    /// on the left nozzle, 2 on the right, one matrix each; the first layer
    /// fixed at filament 2, then a layer of all three.
    #[test]
    fn a_layer_is_ordered_nozzle_by_nozzle_from_the_fixed_first_layer() {
        let left = vec![
            vec![0.0, 290.0, 650.0],
            vec![740.0, 0.0, 750.0],
            vec![650.0, 550.0, 0.0],
        ];
        let right = vec![
            vec![0.0, 240.0, 340.0],
            vec![240.0, 0.0, 710.0],
            vec![540.0, 60.0, 0.0],
        ];
        let flush = [left, right];
        let map = [0, 0, 1];
        let grouping = GroupResult::create(map.to_vec(), two_nozzles(), vec![0, 1, 2]).unwrap();
        let layers = vec![vec![2], vec![0, 1, 2]];
        let fixed = |layer: usize| (layer == 0).then(|| vec![3]);
        let (_, sequences) = reorder_for_nozzles(&[0, 1, 2], &grouping, &layers, &flush, &fixed);
        assert_eq!(sequences, [vec![2], vec![2, 0, 1]]);
        assert_eq!(rule_flush(&sequences[1], &map, &flush, [-1, 2]), 290.0);
        // No order of the layer flushes less.
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            assert!(
                rule_flush(&sequences[1], &map, &flush, [-1, 2])
                    <= rule_flush(&order, &map, &flush, [-1, 2])
            );
        }
    }

    /// A nozzle's answer for a layer is kept by the layer's filaments and
    /// the filament in hand (`caches` in
    /// `reorder_filaments_for_minimum_flush_volume_base`): met again in
    /// another order, the layer gets the first answer, though the solver,
    /// whose ties fall by the order it is handed, would give another.
    #[test]
    fn a_layer_met_again_takes_the_answer_it_got_before() {
        let mut matrix = vec![vec![45.0; 4]; 4];
        for (at, row) in matrix.iter_mut().enumerate() {
            row[at] = 0.0;
        }
        let grouping = GroupResult::create(vec![0; 4], two_nozzles(), vec![0, 1, 2, 3]).unwrap();
        let layers = vec![vec![3], vec![0, 1, 2], vec![3], vec![2, 1, 0]];
        let flush = [matrix.clone(), matrix.clone()];
        let (_, sequences) =
            reorder_for_nozzles(&[0, 1, 2, 3], &grouping, &layers, &flush, &|_| None);
        assert_eq!(sequences[1], [2, 1, 0]);
        assert_eq!(sequences[3], [2, 1, 0]);
        // Uncached, the last layer from 3 would come out the other way.
        let (order, _) = extruders_order(&matrix, &[2, 1, 0], Some(3));
        assert_eq!(order, [0, 1, 2]);
    }

    /// More than twenty filaments on one nozzle are taken greedily, the
    /// cheapest next each time (`solve_extruder_order_with_greedy`).
    #[test]
    fn more_than_twenty_filaments_are_taken_greedily() {
        let count = 21;
        let matrix: Vec<Vec<f32>> = (0..count)
            .map(|from| {
                (0..count)
                    .map(|to| match to {
                        _ if to == from => 0.0,
                        _ if to == from + 1 => 1.0,
                        _ => 100.0,
                    })
                    .collect()
            })
            .collect();
        let tools: Vec<usize> = (0..count).rev().collect();
        // From nothing in hand: the first handed, then the cheapest each
        // time, a tie to the first handed (the dynamic programme would go
        // 0, 1, ... 20 for 20).
        let (order, cost) = extruders_order(&matrix, &tools, None);
        assert_eq!(order, tools);
        assert_eq!(cost, 2000.0);
        let (order, cost) = extruders_order(&matrix, &tools, Some(0));
        assert_eq!(order, (0..count).collect::<Vec<_>>());
        assert_eq!(cost, 20.0);
    }

    /// Sixteen filaments on one nozzle and one on the other, 200 layers of
    /// them, a quarter of the layers one short (the C++ fix's timing case):
    /// every layer comes back whole, and a nozzle's order is cached.
    #[test]
    fn sixteen_filaments_on_one_nozzle_come_back_whole() {
        let count = 17;
        let matrix = |e: usize| -> Vec<Vec<f32>> {
            (0..count)
                .map(|from| {
                    (0..count)
                        .map(|to| {
                            if from == to {
                                0.0
                            } else {
                                (50 + (from * 37 + to * 11 + e * 5) % 500) as f32
                            }
                        })
                        .collect()
                })
                .collect()
        };
        let mut map = vec![0; count];
        map[count - 1] = 1;
        let used: Vec<u32> = (0..count as u32).collect();
        let grouping = GroupResult::create(map, two_nozzles(), used.clone()).unwrap();
        let mut layers = vec![used.clone(); 200];
        for layer in (0..layers.len()).step_by(4) {
            layers[layer].remove(layer / 4 % count);
        }
        let (_, sequences) =
            reorder_for_nozzles(&used, &grouping, &layers, &[matrix(0), matrix(1)], &|_| {
                None
            });
        assert_eq!(sequences.len(), layers.len());
        for (got, asked) in sequences.iter().zip(&layers) {
            let mut got = got.clone();
            got.sort_unstable();
            assert_eq!(&got, asked);
        }
    }

    fn lookahead_matrix() -> Vec<Vec<f32>> {
        vec![
            vec![0.0, 10.0, 15.0, 50.0],
            vec![50.0, 0.0, 10.0, 0.0],
            vec![50.0, 10.0, 0.0, 100.0],
            vec![50.0, 50.0, 50.0, 0.0],
        ]
    }

    /// `reorder_filaments_for_minimum_flush_volume` (ToolOrderUtils.cpp
    /// l.1127) looks one layer ahead where both layers print five
    /// filaments or fewer on the extruder: from 0, 1, 2 flushes least on
    /// its own layer (20 against 25), but 2, 1 leaves 1 in hand, from which
    /// the next layer's 3 costs nothing (25 against 120). Its cache keys on
    /// the next layer too: the same layer from the same filament before a
    /// layer of 0 is 1, 2 (70 against 75).
    #[test]
    fn an_extruder_looks_one_layer_ahead() {
        let matrix = lookahead_matrix();
        let flush = [matrix.clone(), matrix.clone()];
        let layers = vec![vec![0], vec![1, 2], vec![3], vec![0], vec![1, 2], vec![0]];
        let (_, ahead) =
            reorder_for_extruders(&[0, 1, 2, 3], &[0, 0, 0, 0], &layers, &flush, &|_| None);
        assert_eq!(ahead[1], [2, 1]);
        assert_eq!(ahead[4], [1, 2]);
        // The per-nozzle routine (l.1553) does not look ahead.
        let grouping = GroupResult::create(vec![0; 4], two_nozzles(), vec![0, 1, 2, 3]).unwrap();
        let (_, nozzle) = reorder_for_nozzles(&[0, 1, 2, 3], &grouping, &layers, &flush, &|_| None);
        assert_eq!(nozzle[1], [1, 2]);
    }

    /// Above five filaments on a layer the extruder's order is the dynamic
    /// programme's, from the filament in hand.
    #[test]
    fn above_five_filaments_an_extruder_does_not_look_ahead() {
        let count = 7;
        let matrix: Vec<Vec<f32>> = (0..count)
            .map(|from| {
                (0..count)
                    .map(|to| {
                        if from == to {
                            0.0
                        } else {
                            (10 + (from * 7 + to * 3) % 23) as f32
                        }
                    })
                    .collect()
            })
            .collect();
        let six: Vec<u32> = (1..7).collect();
        let layers = vec![vec![0], six.clone(), vec![0]];
        let (_, got) = reorder_for_extruders(
            &(0..7).collect::<Vec<u32>>(),
            &[0; 7],
            &layers,
            &[matrix.clone(), matrix.clone()],
            &|_| None,
        );
        let tools: Vec<usize> = six.iter().map(|f| *f as usize).collect();
        let (expected, _) = extruders_order(&matrix, &tools, Some(0));
        assert_eq!(
            got[1],
            expected.iter().map(|f| *f as u32).collect::<Vec<_>>()
        );
    }

    /// Two extruders: the one printed last goes first on the next layer;
    /// a fixed first layer is kept and the extruders go on from it.
    #[test]
    fn extruders_take_turns_from_a_fixed_first_layer() {
        let matrix = lookahead_matrix();
        let flush = [matrix.clone(), matrix.clone()];
        // 0 and 1 on the left, 2 and 3 on the right.
        let layers = vec![vec![2, 0], vec![0, 1, 2, 3], vec![0, 1, 2, 3]];
        let fixed = |layer: usize| (layer == 0).then(|| vec![3, 1]);
        let (_, got) = reorder_for_extruders(&[0, 1, 2, 3], &[0, 0, 1, 1], &layers, &flush, &fixed);
        assert_eq!(got[0], [2, 0]);
        // Ended on the left: left first (0 in hand: 0, 1), then the right
        // (2 in hand: 2, 3), then the right first.
        assert_eq!(got[1], [0, 1, 2, 3]);
        assert_eq!(got[2][..2], [3, 2]);
    }

    /// The look-ahead is one layer wide and breaks a tie by the first
    /// permutation (fixture h2_mix_four_2: every filament on one extruder,
    /// 199 mm³ forward, 639 back). From 2, with all four on this layer
    /// and the next, 2, 0, 1, 3 and 2, 3, 0, 1 each come to 2074 mm³ with
    /// their best next layer and six changes; the first wins, and every
    /// layer then costs 1037, where the plain solver also finds a layer of
    /// 0, 1, 2, 3 (597) every fourth layer. As the reference's routine
    /// does; on the plate 21,379 -> 22,259 mm³.
    #[test]
    fn the_look_ahead_takes_the_first_of_a_tie() {
        let matrix: Vec<Vec<f32>> = (0..4)
            .map(|from| {
                (0..4)
                    .map(|to| match to {
                        _ if to == from => 0.0,
                        _ if to > from => 199.0,
                        _ => 639.0,
                    })
                    .collect()
            })
            .collect();
        let all = [0, 1, 2, 3];
        let (order, cost) = order_looking_ahead(&matrix, &all, &all, Some(2));
        assert_eq!(order, [2, 0, 1, 3]);
        assert_eq!(cost, 1037.0);
        let mut other = [2u32, 3, 0, 1];
        let total = |order: &[u32]| {
            let mut previous = 2;
            let mut sum = 0.0f32;
            for f in order {
                sum += matrix[previous as usize][*f as usize];
                previous = *f;
            }
            sum
        };
        assert_eq!(total(&other), 1037.0);
        other.sort_unstable();
        assert_eq!(other, all);
    }

    #[test]
    fn permutations_come_in_lexicographic_order() {
        let mut items = vec![1, 2, 3];
        let mut seen = vec![items.clone()];
        while next_permutation(&mut items) {
            seen.push(items.clone());
        }
        assert_eq!(seen.len(), 6);
        assert_eq!(seen[1], [1, 3, 2]);
        assert_eq!(items, [1, 2, 3]);
    }
}
