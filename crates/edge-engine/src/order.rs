//! The order tools are taken in on each layer, as the reference has it
//! (ToolOrdering.cpp).
//!
//! On the first layer, the tool whose smallest piece is largest goes
//! first, so that the pieces most likely to come loose are laid last, on a
//! bed the others have already been across; a plate may set its own order
//! instead (`first_layer_print_sequence`). On every later layer the tool
//! in hand goes first and the rest follow in the order that wastes least
//! on the prime tower. On a printer with a nozzle for each filament every
//! change costs the same, so every order ties, and which one comes out is
//! a matter of how the reference's solver breaks ties. That is ported as
//! it stands, with the order it is handed the tools in, because the tower
//! is one layer taller or shorter by it.

use std::collections::HashMap;

/// The first layer's order: `pieces` gives, for each tool on it in
/// ascending order, the area of its smallest piece. Largest first; of two
/// the same, the lower number first (`generate_first_layer_tool_order`).
/// The areas are compared exactly, as the reference compares them: the
/// pieces are cut as the reference cuts them, in their object's frame
/// with its single-precision corners (`edge_mesh::slicing`), so copies of
/// one object tie to the unit, and parts that are the same in the file
/// but stand at different places in their object come out as the
/// reference has them (on m2b_three the block at the right end is larger
/// by 0.00016 mm² than the one at the left, both there and here).
/// `sequence` is the plate's own order, as filament numbers from 1, and
/// `number` gives a tool's filament number; it is followed when it names
/// every tool on the layer (`apply_first_layer_order`).
pub(crate) fn first_layer(
    pieces: &[(usize, f64)],
    sequence: &[u32],
    number: impl Fn(usize) -> u32,
) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::with_capacity(pieces.len());
    for (tool, area) in pieces {
        let smaller = |other: &usize| {
            pieces
                .iter()
                .find(|(tool, _)| tool == other)
                .is_some_and(|(_, theirs)| theirs < area)
        };
        let at = order.iter().position(smaller).unwrap_or(order.len());
        order.insert(at, *tool);
    }
    let place = |tool: &usize| sequence.iter().position(|named| *named == number(*tool));
    if sequence.len() >= order.len() && order.iter().all(|tool| place(tool).is_some()) {
        order.sort_by_key(place);
    }
    order
}

/// The shortest way through all of `tools` starting from `start`, when
/// going from tool `a` to tool `b` costs `cost(a, b)`: the reference's
/// `solve_extruder_order`, kept step for step so that ties fall as its
/// do (`edge_group::solve_order`, which the filament grouping shares).
/// `start` need not be among the tools; it is then where the way begins
/// and is left out of the answer.
pub(crate) fn solve(
    cost: impl Fn(usize, usize) -> f32,
    tools: &[usize],
    start: Option<usize>,
) -> Vec<usize> {
    edge_group::solve_order(cost, tools, start).0
}

/// The lists the solver is handed for the layers above the first: the
/// reference first brings to the front of each layer's list the tool the
/// list before it ended with (`reorder_extruders`).
pub(crate) fn handed(first: &[usize], later: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut handed: Vec<Vec<usize>> = Vec::with_capacity(later.len());
    let mut ended = first.last().copied();
    for tools in later {
        let mut list = tools.clone();
        if let Some(at) = ended.and_then(|ended| list.iter().position(|tool| *tool == ended))
            && at > 0
        {
            list[..=at].rotate_right(1);
        }
        ended = list.last().copied().or(ended);
        handed.push(list);
    }
    handed
}

/// Every layer's order. `first` is the first layer's, already settled;
/// `later[i]` is the tools on layer `i + 1` in ascending order, empty for
/// a layer with nothing on it, which keeps the tool in hand.
pub(crate) fn layers(
    first: Vec<usize>,
    later: &[Vec<usize>],
    cost: impl Fn(usize, usize) -> f32,
) -> Vec<Vec<usize>> {
    let handed = handed(&first, later);
    // Then the solver, layer by layer from the tool in hand; a set of
    // tools met again from the same tool gets the answer it got before
    // (`reorder_extruders_for_minimum_flush_volume`).
    let mut known: HashMap<(Vec<usize>, Option<usize>), Vec<usize>> = HashMap::new();
    let mut in_hand = first.last().copied();
    let mut orders = Vec::with_capacity(later.len() + 1);
    orders.push(first);
    for (tools, list) in later.iter().zip(handed) {
        if tools.is_empty() {
            orders.push(Vec::new());
            continue;
        }
        let order = known
            .entry((tools.clone(), in_hand))
            .or_insert_with(|| solve(&cost, &list, in_hand))
            .clone();
        in_hand = order.last().copied().or(in_hand);
        orders.push(order);
    }
    orders
}

/// Every layer's order on a printer of two extruders whose filaments the
/// grouping has placed (the H2D, H2C, X2D), as filaments from 0, by the
/// routine Bambu Studio orders such a printer with (the rule of C++ fix PR
/// aceRage/EdgeSlicer#1169, work/cppfix/order/ORDER_RULE.md, after the
/// lead's reading of Bambu Studio 02.09.01.70):
///
/// - an extruder with a nozzle rack (`rack`: `extruder_max_nozzle_count`
///   above one on some extruder, the H2C): nozzle by nozzle,
///   `reorder_filaments_for_multi_nozzle_extruder` (pinned
///   GCode/ToolOrderUtils.cpp l.1553-1710, `edge_group::reorder_for_nozzles`,
///   the routine the grouping scores its maps with): each nozzle's
///   filaments from the filament that nozzle last held, by its extruder's
///   matrix, without looking ahead; one extruder's nozzles after the
///   other's, the nozzle used last first;
/// - otherwise (the H2D, X2D): extruder by extruder,
///   `reorder_filaments_for_minimum_flush_volume` (l.1127-1306,
///   `edge_group::reorder_for_extruders`): each extruder's filaments from
///   the filament it last printed, looking one layer ahead where the
///   extruder prints five filaments or fewer on this layer and the next.
///
/// Either way the extruder printed last goes first on the next layer. On
/// the H2D and X2D the grouping still scores its maps by the per-nozzle
/// routine (FilamentGroup.cpp l.157), so the map is chosen by one routine
/// and printed by the other: on purpose, as Bambu Studio does.
/// `flush` is each extruder's matrix, multiplier applied, as the grouping
/// reads it. `lists` is each layer's filaments as the solver is handed
/// them (the first layer in its settled order, the others as `handed`
/// turns them; ties fall by it); the first layer keeps its order and the
/// routines go on from it. Nothing is loaded before it, as the reference
/// has it (an empty `NozzleStatusRecorder` / `nozzle_status`).
///
/// A deliberate difference from the pinned reference, whose layers are
/// ordered by one chain over the whole layer whatever nozzle a filament
/// prints from (ToolOrdering.cpp l.2266-2318,
/// `reorder_extruders_for_minimum_flush_volume`, `get_extruders_order`):
/// C++ fix PR aceRage/EdgeSlicer#1169 (`ToolOrdering::order_layers_by_nozzle`)
/// brings the reference to this order. As there, none (the chain orders
/// the plate) where a filament is numbered 32 or more (the routines key
/// their caches on a bit per filament), has no nozzle or lies outside its
/// extruder's matrix, where there are not two matrices, or where a layer
/// does not come back as the same filaments.
pub(crate) fn by_nozzle(
    grouping: &edge_group::GroupResult,
    flush: &[edge_group::FlushMatrix],
    lists: &[Vec<u32>],
    rack: bool,
) -> Option<Vec<Vec<u32>>> {
    if flush.len() != 2 || lists.is_empty() {
        return None;
    }
    let used: Vec<u32> = (lists.iter().flatten().copied())
        .collect::<std::collections::BTreeSet<u32>>()
        .into_iter()
        .collect();
    for filament in &used {
        let nozzle = grouping.nozzle_for(i64::from(*filament))?;
        let matrix = usize::try_from(nozzle.extruder_id)
            .ok()
            .and_then(|extruder| flush.get(extruder))?;
        if *filament >= 32 || *filament as usize >= matrix.len() {
            return None;
        }
    }
    // The first layer is fixed: its order is the bed's (`first_layer`)
    // or the plate's own, and the reference hands the routine the same
    // (`order_layers_by_nozzle`'s `fixed`). The plate's orders for its
    // other layers (`other_layers_print_sequence`), which the reference
    // keeps fixed, are not followed by this engine on any printer (see
    // the notes the slice writes), and cyclic tool changes fix every
    // layer, so the caller does not come here with them.
    let first: Vec<i64> = lists[0].iter().map(|f| i64::from(*f) + 1).collect();
    let fixed = |layer: usize| (layer == 0 && !first.is_empty()).then(|| first.clone());
    let (_, sequences) = if rack {
        edge_group::reorder_for_nozzles(&used, grouping, lists, flush, &fixed)
    } else {
        let extruders: Vec<i32> = (used.iter())
            .map(|f| {
                grouping
                    .nozzle_for(i64::from(*f))
                    .map_or(-1, |n| n.extruder_id)
            })
            .collect();
        edge_group::reorder_for_extruders(&used, &extruders, lists, flush, &fixed)
    };
    if sequences.len() != lists.len() {
        return None;
    }
    for (got, asked) in sequences.iter().zip(lists) {
        let (mut got, mut asked) = (got.clone(), asked.clone());
        got.sort_unstable();
        asked.sort_unstable();
        if got != asked {
            return None;
        }
    }
    Some(sequences)
}

/// The fixed order of cyclic tool changes (`toolchange_ordering`
/// "cyclic"), as filament numbers from 1: `toolchange_cyclic_order`'s
/// numbers ("3,2,1,4"), each kept once and only if it is one of the
/// project's `count` filaments; anything else in it is passed over
/// (ToolOrdering.cpp `parse_cyclic_order`). Where that leaves none, the
/// first range of the plate's own order for its other layers
/// (`other_layers_print_sequence`, read the same way) that names one
/// (`cyclic_order_from_customized_sequence`); empty for none at all.
pub(crate) fn cyclic_sequence(text: &str, other_layers: &[Vec<i64>], count: usize) -> Vec<u32> {
    let mut sequence: Vec<u32> = Vec::new();
    let keep = |number: i64, into: &mut Vec<u32>| {
        if let Ok(number) = u32::try_from(number)
            && number >= 1
            && number as usize <= count
            && !into.contains(&number)
        {
            into.push(number);
        }
    };
    for token in text.split(',') {
        // `std::stoi`: leading space skipped, trailing space allowed,
        // anything else after the number and the token is dropped.
        let token = token.trim_start().trim_end_matches([' ', '\t', '\r', '\n']);
        if let Ok(number) = token.parse::<i32>() {
            keep(i64::from(number), &mut sequence);
        }
    }
    if sequence.is_empty() {
        for range in other_layers {
            for &number in range {
                keep(number, &mut sequence);
            }
            if !sequence.is_empty() {
                break;
            }
        }
    }
    sequence
}

/// The plate's own order for its other layers, one list of filament
/// numbers per range of layers (ParameterUtils.cpp
/// `get_other_layers_print_sequence`): `values` in `ranges` groups, each
/// its first and last layer and then the filaments. None where the
/// count does not divide them into groups of three or more.
pub(crate) fn other_layers_sequences(ranges: i64, values: &[i64]) -> Vec<Vec<i64>> {
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
        .map(|group| group[2..].to_vec())
        .collect()
}

/// A layer's order under cyclic tool changes: its `tools` (as the layer
/// lists them, ascending) in the order of `sequence`, those it does not
/// name after them, ascending; `number` gives a tool's filament number
/// (ToolOrdering.cpp `apply_cyclic_order`).
pub(crate) fn cyclic(
    tools: &[usize],
    sequence: &[u32],
    number: impl Fn(usize) -> u32,
) -> Vec<usize> {
    let mut order = tools.to_vec();
    order.sort_by_key(|tool| number(*tool));
    order.sort_by_key(|tool| {
        sequence
            .iter()
            .position(|named| *named == number(*tool))
            .unwrap_or(sequence.len())
    });
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(_: usize, _: usize) -> f32 {
        45.0
    }

    fn grouping(map: Vec<i32>, used: Vec<u32>) -> edge_group::GroupResult {
        let nozzles = edge_group::nozzle_list(&[
            ("0.4".into(), edge_group::VolumeType::STANDARD, 0, 1),
            ("0.4".into(), edge_group::VolumeType::STANDARD, 1, 1),
        ]);
        edge_group::GroupResult::create(map, nozzles, used).unwrap()
    }

    /// The left nozzle's matrix of the C++ fix's case, on both nozzles.
    fn matrix() -> Vec<Vec<f32>> {
        vec![
            vec![0.0, 290.0, 650.0],
            vec![740.0, 0.0, 750.0],
            vec![650.0, 550.0, 0.0],
        ]
    }

    #[test]
    fn nozzle_by_nozzle_keeps_the_first_layer_and_turns_no_nozzle_back() {
        // 0 and 1 on the left, 2 on the right. The first layer starts on
        // the left and keeps its order; the next starts on the extruder the
        // first ended on and changes extruder once.
        let grouping = grouping(vec![0, 0, 1], vec![0, 1, 2]);
        let flush = vec![matrix(), matrix()];
        let lists = vec![vec![0, 1], vec![2, 0, 1], vec![0, 1, 2]];
        let orders = by_nozzle(&grouping, &flush, &lists, true).unwrap();
        assert_eq!(orders[0], [0, 1]);
        assert_eq!(orders[1], [1, 0, 2]);
        // From 0 on the left and 2 on the right, ending on the right.
        assert_eq!(orders[2], [2, 0, 1]);
    }

    #[test]
    fn a_change_onto_the_other_nozzle_is_not_charged() {
        // 0 and 1 on the left, 2 on the right; the first layer leaves 1 on
        // the left and 2 on the right. The chain over the whole layer
        // charges 2 -> 0 and 2 -> 1 though the right nozzle keeps 2, and
        // takes 2, 0, 1 (10 + 500), which flushes 1 -> 0 -> 1 on the left:
        // 600. Nozzle by nozzle: 2, then 1, 0 on the left: 100.
        let matrix = vec![
            vec![0.0, 500.0, 300.0],
            vec![100.0, 0.0, 300.0],
            vec![10.0, 1000.0, 0.0],
        ];
        let grouping = grouping(vec![0, 0, 1], vec![0, 1, 2]);
        let lists = vec![vec![1, 2], vec![2, 0, 1]];
        let orders =
            by_nozzle(&grouping, &[matrix.clone(), matrix.clone()], &lists, false).unwrap();
        assert_eq!(orders, [vec![1, 2], vec![2, 1, 0]]);
        let chain = layers(vec![1, 2], &[vec![0, 1, 2]], |a, b| matrix[a][b]);
        assert_eq!(chain[1], [2, 0, 1]);
    }

    #[test]
    fn the_h2d_looks_a_layer_ahead_and_the_rack_does_not() {
        // Every filament on the left. From 0 the layer 1, 2 flushes least
        // on its own (20 against 25), but 2, 1 leaves 1 in hand, from which
        // the next layer's 3 costs nothing.
        let matrix = vec![
            vec![0.0, 10.0, 15.0, 50.0],
            vec![50.0, 0.0, 10.0, 0.0],
            vec![50.0, 10.0, 0.0, 100.0],
            vec![50.0, 50.0, 50.0, 0.0],
        ];
        let grouping = grouping(vec![0; 4], vec![0, 1, 2, 3]);
        let flush = vec![matrix.clone(), matrix];
        let lists = vec![vec![0], vec![1, 2], vec![3]];
        let h2d = by_nozzle(&grouping, &flush, &lists, false).unwrap();
        let rack = by_nozzle(&grouping, &flush, &lists, true).unwrap();
        assert_eq!(h2d[1], [2, 1]);
        assert_eq!(rack[1], [1, 2]);
    }

    #[test]
    fn plates_the_routine_cannot_take_keep_the_chain() {
        let flush = vec![matrix(), matrix()];
        let lists = vec![vec![0, 1], vec![0, 1, 2]];
        // A filament with no nozzle.
        let unplaced = grouping(vec![0, 0], vec![0, 1]);
        assert!(by_nozzle(&unplaced, &flush, &lists, false).is_none());
        // A filament outside its extruder's matrix.
        let placed = grouping(vec![0, 0, 1], vec![0, 1, 2]);
        let short = vec![matrix(), vec![vec![0.0; 2]; 2]];
        assert!(by_nozzle(&placed, &short, &lists, false).is_none());
        // Not two matrices.
        assert!(by_nozzle(&placed, &flush[..1], &lists, true).is_none());
        // A filament numbered 32 or more: the routine keys on a bit each.
        let mut map = vec![0; 33];
        map[32] = 1;
        let big = grouping(map, vec![0, 32]);
        let wide: Vec<Vec<f32>> = vec![vec![10.0; 33]; 33];
        let far = vec![vec![0], vec![32, 0]];
        assert!(by_nozzle(&big, &[wide.clone(), wide], &far, false).is_none());
        assert!(by_nozzle(&placed, &flush, &[], true).is_none());
    }

    #[test]
    fn on_the_bed_the_tool_with_the_largest_smallest_piece_goes_first() {
        let own = |tool: usize| tool as u32 + 1;
        // Two blocks, the second the smaller: the first goes first.
        assert_eq!(first_layer(&[(0, 400.0), (1, 200.0)], &[], own), [0, 1]);
        // The first the smaller: the second goes first.
        assert_eq!(first_layer(&[(0, 200.0), (1, 400.0)], &[], own), [1, 0]);
        // Of two the same, the lower number; two that differ at all are
        // not the same.
        assert_eq!(
            first_layer(&[(0, 300.0), (1, 500.0), (2, 300.0), (3, 500.0)], &[], own),
            [1, 3, 0, 2]
        );
        assert_eq!(
            first_layer(&[(0, 615.833120), (1, 615.833186)], &[], own),
            [1, 0]
        );
        assert_eq!(first_layer(&[(0, 615.0), (1, 615.9)], &[], own), [1, 0]);
        // A plate's own order, in filament numbers, is followed when it
        // names every tool on the layer.
        let pieces = [(1, 600.0), (2, 190.0), (3, 620.0), (4, 610.0), (5, 615.0)];
        assert_eq!(
            first_layer(&pieces, &[1, 2, 6, 4, 5, 3, 7], own),
            [1, 5, 3, 4, 2]
        );
        // One that leaves a tool out, or is shorter than the layer, is not.
        assert_eq!(first_layer(&pieces, &[2, 6, 4, 5, 9], own), [3, 5, 4, 1, 2]);
        assert_eq!(first_layer(&pieces, &[0], own), [3, 5, 4, 1, 2]);
    }

    #[test]
    fn the_solver_breaks_ties_as_the_reference_s_does() {
        // With every change the same price, it starts from the tool in
        // hand, ends at the tool listed after it, and takes the rest from
        // the back of the list forward.
        assert_eq!(solve(same, &[1, 2, 3, 4, 5], Some(2)), [2, 5, 4, 3, 1]);
        assert_eq!(solve(same, &[1, 2, 3, 4, 5], Some(1)), [1, 5, 4, 3, 2]);
        assert_eq!(solve(same, &[5, 1, 2, 3, 4], Some(1)), [1, 4, 3, 2, 5]);
        assert_eq!(solve(same, &[4, 1, 2, 3, 5], Some(5)), [5, 4, 3, 2, 1]);
        // With no tool in hand it starts from the first listed.
        assert_eq!(solve(same, &[1, 2, 3], None), [1, 3, 2]);
        // A tool in hand that the layer does not use is where the way
        // starts, and is not in it.
        assert_eq!(solve(same, &[2], Some(5)), [2]);
        assert_eq!(solve(same, &[1, 2, 3], Some(7)), [3, 2, 1]);
        assert_eq!(solve(same, &[], Some(1)), Vec::<usize>::new());
        // Where prices differ the cheapest way wins: from 0, going to 2
        // first costs less than going to 1 first.
        let dear = |a: usize, b: usize| if (a, b) == (0, 1) { 100.0 } else { 10.0 };
        assert_eq!(solve(dear, &[0, 1, 2], Some(0)), [0, 2, 1]);
    }

    #[test]
    fn cyclic_layers_keep_one_order() {
        // Numbers past the project's, repeated or not numbers at all are
        // passed over.
        assert_eq!(cyclic_sequence("7,3,x,2,9,3", &[], 7), [7, 3, 2]);
        assert_eq!(cyclic_sequence(" 2 , 1", &[], 2), [2, 1]);
        assert_eq!(cyclic_sequence("2x,0,-1,1.5", &[], 4), Vec::<u32>::new());
        // With none, the first range of the plate's own order that names
        // one.
        let other = other_layers_sequences(2, &[2, 4, 9, 9, 5, 9, 3, 1]);
        assert_eq!(other, [vec![9, 9], vec![3, 1]]);
        assert_eq!(cyclic_sequence("", &other, 4), [3, 1]);
        assert_eq!(cyclic_sequence("2", &other, 4), [2]);
        // A count that does not divide them is no order.
        assert!(other_layers_sequences(2, &[1, 2, 3, 4, 5]).is_empty());
        assert!(other_layers_sequences(0, &[1, 2, 3]).is_empty());
        // The layer's tools in that order, the rest after, ascending.
        let own = |tool: usize| tool as u32 + 1;
        assert_eq!(cyclic(&[1, 2, 3], &[7, 3, 2], own), [2, 1, 3]);
        assert_eq!(cyclic(&[0, 1, 2, 3], &[], own), [0, 1, 2, 3]);
        assert_eq!(cyclic(&[0, 3, 5], &[4], own), [3, 0, 5]);
    }

    #[test]
    fn layers_come_out_as_in_the_reference_s_files() {
        // `mcbrim_test.3mf` plate 2: five tools up to layer 5, a sixth
        // from layer 6, one alone at the top. Tools are numbered as in
        // the G-code, from 0.
        let five = vec![1, 2, 3, 4, 5];
        let six = vec![1, 2, 3, 4, 5, 6];
        let mut later = vec![five; 5];
        later.extend(vec![six; 5]);
        later.push(vec![2]);
        let orders = layers(vec![1, 5, 3, 4, 2], &later, same);
        let wanted: [&[usize]; 12] = [
            &[1, 5, 3, 4, 2],
            &[2, 5, 4, 3, 1],
            &[1, 4, 3, 2, 5],
            &[5, 4, 3, 2, 1],
            &[1, 4, 3, 2, 5],
            &[5, 4, 3, 2, 1],
            &[1, 6, 4, 3, 2, 5],
            &[5, 6, 4, 3, 2, 1],
            &[1, 6, 4, 3, 2, 5],
            &[5, 6, 4, 3, 2, 1],
            &[1, 6, 4, 3, 2, 5],
            &[2],
        ];
        assert_eq!(orders, wanted);
        // Plate 1 of the same project: three tools, then a fourth.
        let mut later = vec![vec![1, 2, 3]; 5];
        later.extend(vec![vec![1, 2, 3, 6]; 5]);
        let orders = layers(vec![1, 3, 2], &later, same);
        let wanted: [&[usize]; 11] = [
            &[1, 3, 2],
            &[2, 3, 1],
            &[1, 2, 3],
            &[3, 2, 1],
            &[1, 2, 3],
            &[3, 2, 1],
            &[1, 6, 2, 3],
            &[3, 6, 2, 1],
            &[1, 6, 2, 3],
            &[3, 6, 2, 1],
            &[1, 6, 2, 3],
        ];
        assert_eq!(orders, wanted);
        // A layer with nothing on it keeps the tool in hand, and the next
        // carries on from it.
        let orders = layers(vec![0, 1], &[vec![], vec![0, 1]], same);
        assert_eq!(orders, [vec![0, 1], vec![], vec![1, 0]]);
    }

    #[test]
    fn two_nozzles_with_a_matrix_each_are_priced_at_the_new_filament_s() {
        // fx/m2c_four_pn_1212 (H2D, filaments 1 and 3 on the left nozzle,
        // 2 and 4 on the right, a 4 × 4 matrix to each): layers 1 to 5.
        let each: [f32; 32] = [
            0., 300., 120., 510., 90., 0., 260., 180., 450., 70., 0., 330., 200., 610., 140., 0.,
            0., 250., 410., 160., 330., 0., 95., 520., 60., 380., 0., 240., 470., 130., 210., 0.,
        ];
        let nozzle = [0, 1, 0, 1];
        let per_nozzle = |from: usize, to: usize| each[nozzle[to] * 16 + from * 4 + to];
        let later = vec![vec![0, 1, 2, 3]; 4];
        let wanted: [&[usize]; 5] = [
            &[1, 2, 0, 3],
            &[3, 1, 0, 2],
            &[2, 3, 1, 0],
            &[0, 2, 3, 1],
            &[1, 0, 3, 2],
        ];
        assert_eq!(layers(vec![1, 2, 0, 3], &later, per_nozzle), wanted);
        // The reference reads the 32 values (as its command line leaves
        // them) as one 5 × 5 square, and its file has these orders.
        let theirs: [f32; 32] = [
            0., 300., 120., 510., 90., 0., 0., 180., 450., 70., 0., 330., 0., 610., 140., 0., 0.,
            250., 0., 160., 330., 0., 95., 520., 0., 380., 0., 240., 470., 130., 210., 0.,
        ];
        let square = |from: usize, to: usize| theirs[from * 5 + to];
        let wanted: [&[usize]; 5] = [
            &[1, 2, 0, 3],
            &[3, 1, 0, 2],
            &[2, 0, 3, 1],
            &[1, 3, 0, 2],
            &[2, 0, 3, 1],
        ];
        assert_eq!(layers(vec![1, 2, 0, 3], &later, square), wanted);
    }
}
