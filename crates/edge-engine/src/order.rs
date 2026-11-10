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

/// Two areas no further apart than this share of the larger are the same.
/// The reference compares them exactly; here, where the same shape cut in
/// two places comes out a hair different, that would leave the order to
/// chance.
const SAME_AREA: f64 = 1e-6;

/// The first layer's order: `pieces` gives, for each tool on it in
/// ascending order, the area of its smallest piece. Largest first; of two
/// the same, the lower number first (`generate_first_layer_tool_order`).
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
                .is_some_and(|(_, theirs)| *theirs < area - SAME_AREA * area.abs())
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
/// do. `start` need not be among the tools; it is then where the way
/// begins and is left out of the answer.
pub(crate) fn solve(
    cost: impl Fn(usize, usize) -> f32,
    tools: &[usize],
    start: Option<usize>,
) -> Vec<usize> {
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
            None => return Vec::new(),
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
    let (mut best, mut end) = (f32::MAX, 0);
    for last in 0..count {
        if all[last] != start && best > least[whole][last] {
            best = least[whole][last];
            end = last;
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
    path
}

/// Every layer's order. `first` is the first layer's, already settled;
/// `later[i]` is the tools on layer `i + 1` in ascending order, empty for
/// a layer with nothing on it, which keeps the tool in hand.
pub(crate) fn layers(
    first: Vec<usize>,
    later: &[Vec<usize>],
    cost: impl Fn(usize, usize) -> f32,
) -> Vec<Vec<usize>> {
    // The reference first brings to the front of each layer's list the
    // tool the list before it ended with (`reorder_extruders`), and it is
    // those lists the solver is handed.
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
        // Of two the same, the lower number; and two are the same that
        // differ by less than a millionth.
        assert_eq!(
            first_layer(&[(0, 300.0), (1, 500.0), (2, 300.0), (3, 500.0)], &[], own),
            [1, 3, 0, 2]
        );
        assert_eq!(
            first_layer(&[(0, 615.833120), (1, 615.833186)], &[], own),
            [0, 1]
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
}
