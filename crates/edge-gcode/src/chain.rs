//! The order to lay a set of open lines in, and which way round each, so
//! that the nozzle travels little between them.
//!
//! A port of the reference's `chain_segments_greedy_constrained_reversals_`
//! and `chain_segments_closest_point` (ShortestPath.cpp), which is what
//! `chain_and_reorder_extrusion_entities` orders fill and supports with.
//! The reference finds near points with a k-d tree; here every point is
//! looked at, and of two equally near the earlier is taken.

use std::collections::BTreeSet;

use edge_geom::Point2;

fn apart(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)
}

/// Chains that have been joined, each known by the lowest of their numbers.
struct Chains {
    same_as: Vec<usize>,
}

impl Chains {
    fn new() -> Self {
        Self { same_as: vec![0] }
    }

    fn next(&mut self) -> usize {
        self.same_as.push(self.same_as.len());
        self.same_as.len() - 1
    }

    fn of(&mut self, chain: usize) -> usize {
        let mut last = chain;
        while self.same_as[last] != last {
            last = self.same_as[last];
        }
        if chain != 0 {
            self.same_as[chain] = last;
        }
        last
    }

    fn merge(&mut self, a: usize, b: usize) -> usize {
        let chain = self.of(a).min(self.of(b));
        self.same_as[a] = chain;
        self.same_as[b] = chain;
        chain
    }
}

/// The order to take lines in and whether each is turned round: `ends`
/// gives each line's first and last point, `reversible` says which may be
/// turned, and `start` is where the nozzle is.
///
/// The shortest hop between two ends is taken first, then the next
/// shortest that neither closes a loop nor joins an end already joined,
/// until the lines make one chain (the multi-fragment way with the
/// travelling salesman). Nothing in that minds which lines may not be
/// turned; if the chain has one of them the wrong way round, the lines
/// are instead taken nearest first from the start, turning only those
/// that may be.
pub(crate) fn chain(
    ends: &[[Point2; 2]],
    reversible: &dyn Fn(usize) -> bool,
    start: Option<Point2>,
) -> Vec<(usize, bool)> {
    let count = ends.len();
    let as_pair = |point: Point2| (point.x as f64, point.y as f64);
    if count == 0 {
        return Vec::new();
    }
    // With the nozzle nowhere yet, the first line is begun where it
    // begins. (The reference, given no start, begins at whichever end
    // its queue happens to leave free.)
    let start = Some(as_pair(start.unwrap_or(ends[0][0])));
    if count == 1 {
        let [head, tail] = ends[0].map(as_pair);
        let turned =
            reversible(0) && start.is_some_and(|start| apart(tail, start) < apart(head, start));
        return vec![(0, turned)];
    }
    // Ends in pairs: a line's first point, then its last.
    let at: Vec<(f64, f64)> = ends.iter().flatten().copied().map(as_pair).collect();
    let closest = |from: (f64, f64), allowed: &dyn Fn(usize) -> bool| -> Option<usize> {
        (0..at.len())
            .filter(|&end| allowed(end))
            .min_by(|&a, &b| apart(at[a], from).total_cmp(&apart(at[b], from)))
    };
    let may_begin = |end: usize| end.is_multiple_of(2) || reversible(end / 2);

    let mut chain_of = vec![0usize; at.len()];
    let mut joined: Vec<Option<usize>> = vec![None; at.len()];
    let mut hop = vec![f64::MAX; at.len()];
    let mut chains = Chains::new();
    let mut first = start.and_then(|start| closest(start, &may_begin));
    if let Some(first) = first {
        hop[first] = 0.0;
        chain_of[first] = chains.next();
    }
    let initial = first;
    // Ends still free, nearest hop first.
    let mut queue: BTreeSet<(u64, usize)> = BTreeSet::new();
    for end in (0..at.len()).filter(|&end| Some(end) != first) {
        let near = closest(at[end], &|other| Some(other) != first && (other ^ end) > 1);
        if let Some(near) = near {
            joined[end] = Some(near);
            hop[end] = apart(at[near], at[end]);
        }
        queue.insert((hop[end].to_bits(), end));
    }

    let mut last = None;
    let mut left = count - 1;
    let mut sound = true;
    while left > 0 {
        let Some(&(key, one)) = queue.first() else {
            sound = false;
            break;
        };
        let Some(other) = joined[one] else {
            sound = false;
            break;
        };
        let (mut valid, mut one_chain, mut other_chain) = (chain_of[other] == 0, 0, 0);
        if valid {
            one_chain = chains.of(chain_of[one ^ 1]);
            other_chain = chains.of(chain_of[other ^ 1]);
            // Both lines already in one chain: joining them closes a loop.
            valid = one_chain != other_chain || one_chain == 0;
        }
        if valid {
            queue.remove(&(key, one));
            queue.remove(&(hop[other].to_bits(), other));
            joined[other] = Some(one);
            hop[other] = hop[one];
            let chain = match (one_chain, other_chain) {
                (0, 0) => chains.next(),
                (0, chain) | (chain, 0) => chain,
                (a, b) => chains.merge(a, b),
            };
            chain_of[one] = chain;
            chain_of[other] = chain;
            left -= 1;
        } else {
            // Its nearest is taken: the next nearest that would not
            // close a loop.
            let own = chains.of(chain_of[one ^ 1]);
            let mut others: Vec<usize> = (0..at.len()).map(|end| chain_of[end ^ 1]).collect();
            for chain in &mut others {
                *chain = chains.of(*chain);
            }
            let near = closest(at[one], &|end| {
                (end ^ one) > 1 && chain_of[end] == 0 && (others[end] != own || own == 0)
            });
            queue.remove(&(key, one));
            match near {
                Some(near) => {
                    joined[one] = Some(near);
                    hop[one] = apart(at[near], at[one]);
                    queue.insert((hop[one].to_bits(), one));
                }
                None => {
                    sound = false;
                    break;
                }
            }
        }
    }
    if sound {
        // Two ends are left, or one beside the start: the chain's ends.
        let mut free = queue.iter().map(|&(_, end)| end);
        if first.is_none() {
            first = free.next();
        }
        last = free.next();
        for end in [first, last].into_iter().flatten() {
            joined[end] = None;
        }
    }

    // Along the chain from an end; `None` when a line would be turned
    // that may not be.
    let walk = |from: usize| -> Option<Vec<(usize, bool)>> {
        let mut order = Vec::with_capacity(count);
        let mut end = Some(from);
        while let Some(at) = end {
            let (line, turned) = (at / 2, at % 2 == 1);
            if (turned && !reversible(line)) || order.len() >= count {
                return None;
            }
            order.push((line, turned));
            end = joined[at ^ 1];
        }
        (order.len() == count).then_some(order)
    };
    if sound && let Some(first) = first {
        if let Some(order) = walk(first) {
            return order;
        }
        // With nowhere to start from, the chain may be walked from its
        // other end.
        if start.is_none()
            && let Some(order) = last.and_then(walk)
        {
            return order;
        }
    }

    // Nearest first from where the chain began, or from the first line.
    let begin = initial.unwrap_or(0);
    let mut used = vec![false; at.len()];
    let mut order = vec![(begin / 2, begin % 2 == 1)];
    used[begin] = true;
    let mut here = begin ^ 1;
    for _ in 1..count {
        used[here] = true;
        let next = closest(at[here], &|end| {
            (end ^ here) > 1 && !used[end] && may_begin(end)
        })
        // Only when every line left would have to be turned and may
        // not: taken as it lies.
        .or_else(|| {
            (0..at.len()).find(|&end| !used[end] && !used[end ^ 1] && end.is_multiple_of(2))
        });
        let Some(next) = next else { break };
        used[next] = true;
        order.push((next / 2, next % 2 == 1));
        here = next ^ 1;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f64, f64), b: (f64, f64)) -> [Point2; 2] {
        [Point2::from_mm(a.0, a.1), Point2::from_mm(b.0, b.1)]
    }

    const ANY: &dyn Fn(usize) -> bool = &|_| true;

    #[test]
    fn lines_are_chained_by_their_closest_ends() {
        // Rungs of a ladder given out of order, all pointing the same
        // way, and further apart the higher they are.
        let rungs = [
            line((0.0, 4.5), (10.0, 4.5)),
            line((0.0, 0.0), (10.0, 0.0)),
            line((0.0, 7.5), (10.0, 7.5)),
            line((0.0, 2.0), (10.0, 2.0)),
        ];
        // From below the first rung's left end: up the ladder, to and fro.
        let order = chain(&rungs, ANY, Some(Point2::from_mm(0.0, -1.0)));
        assert_eq!(order, [(1, false), (3, true), (0, false), (2, true)]);
        // From above the last one's right end: down again.
        let order = chain(&rungs, ANY, Some(Point2::from_mm(10.0, 8.5)));
        assert_eq!(order, [(2, true), (0, false), (3, true), (1, false)]);
        // Every line once, wherever the start is, or with none.
        for start in [None, Some(Point2::from_mm(5.0, 3.0))] {
            let mut lines: Vec<usize> = chain(&rungs, ANY, start).iter().map(|l| l.0).collect();
            lines.sort_unstable();
            assert_eq!(lines, [0, 1, 2, 3]);
        }
        // With no start the first line given is begun where it begins.
        assert_eq!(chain(&rungs, ANY, None)[0], (0, false));
    }

    #[test]
    fn the_shortest_hops_are_joined_first() {
        // Two pairs of lines: within a pair the ends are 1 mm apart, and
        // the pairs 3 mm. Nearest first from a start beside the gap
        // would take one line of each pair and come back for the others.
        let lines = [
            line((0.0, 0.0), (4.0, 0.0)),
            line((5.0, 0.0), (9.0, 0.0)),
            line((12.0, 0.0), (16.0, 0.0)),
            line((17.0, 0.0), (21.0, 0.0)),
        ];
        let order = chain(&lines, ANY, Some(Point2::from_mm(0.0, 1.0)));
        assert_eq!(order, [(0, false), (1, false), (2, false), (3, false)]);
        // Whatever the start, the pairs stay together.
        let order = chain(&lines, ANY, Some(Point2::from_mm(10.0, 1.0)));
        let at = |line: usize| order.iter().position(|step| step.0 == line).unwrap();
        assert_eq!(at(0).abs_diff(at(1)), 1);
        assert_eq!(at(2).abs_diff(at(3)), 1);
    }

    #[test]
    fn lines_that_may_not_be_turned_are_not() {
        let rungs = [
            line((0.0, 0.0), (10.0, 0.0)),
            line((0.0, 2.0), (10.0, 2.0)),
            line((0.0, 4.0), (10.0, 4.0)),
        ];
        let fixed: &dyn Fn(usize) -> bool = &|_| false;
        let order = chain(&rungs, fixed, Some(Point2::from_mm(0.0, -1.0)));
        assert_eq!(order, [(0, false), (1, false), (2, false)]);
        // One alone is turned only when it may be and its far end is nearer.
        let one = [rungs[0]];
        let far = Some(Point2::from_mm(11.0, 0.0));
        assert_eq!(chain(&one, ANY, far), [(0, true)]);
        assert_eq!(chain(&one, fixed, far), [(0, false)]);
        assert_eq!(chain(&one, ANY, None), [(0, false)]);
        assert!(chain(&[], ANY, None).is_empty());
    }
}
