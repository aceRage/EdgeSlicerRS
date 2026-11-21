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
