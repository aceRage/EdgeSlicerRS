//! The order the copies on a plate are printed in on a layer, as the
//! reference chains them: ShortestPath.cpp `chain_print_object_instances`,
//! which hands each copy's middle to `chain_segments_greedy` as a segment
//! whose two ends are the one point.
//!
//! A port of `chain_segments_greedy_constrained_reversals_` (with no
//! reversal constraints) together with what decides its ties and, given
//! no start, which end of the chain it starts from: the k-d tree it finds
//! the nearest end with (KDTreeIndirect.hpp: `build_recursive`,
//! `partition_input`, `find_closest_points`, where of two ends equally
//! near the later visited wins) and its binary heap
//! (MutablePriorityQueue.hpp). Copies stand on a regular grid often
//! enough (calibration prints, copies arranged in rows) that the ties
//! decide the order; [`crate::chain`] takes the earlier of two equal
//! points and, given no start, starts at the first line, which is right
//! for fill but not for this.

/// `libslic3r.h` `EPSILON`, which the k-d tree's descent widens its
/// search radius by.
const EPSILON: f64 = 1e-4;

const NONE: usize = usize::MAX;

/// KDTreeIndirect<2, double>: a balanced tree over the points' indices,
/// built by splitting at the median of x, then y, and so on.
struct KdTree<'a> {
    points: &'a [[f64; 2]],
    nodes: Vec<usize>,
}

impl<'a> KdTree<'a> {
    fn new(points: &'a [[f64; 2]]) -> Self {
        let mut tree = Self {
            points,
            nodes: Vec::new(),
        };
        if !points.is_empty() {
            tree.nodes = vec![NONE; (points.len() + 1).next_power_of_two()];
            let mut input: Vec<usize> = (0..points.len()).collect();
            tree.build(&mut input, 0, 0, 0, points.len() - 1);
        }
        tree
    }

    fn coordinate(&self, index: usize, dimension: usize) -> f64 {
        self.points[index][dimension]
    }

    fn build(
        &mut self,
        input: &mut [usize],
        node: usize,
        dimension: usize,
        left: usize,
        right: usize,
    ) {
        if left > right {
            return;
        }
        if left == right {
            self.nodes[node] = input[left];
            return;
        }
        let center = (left + right) / 2;
        self.partition(input, dimension, left, right, center);
        self.nodes[node] = input[center];
        let next = (dimension + 1) % 2;
        if center > left {
            self.build(input, node * 2 + 1, next, left, center - 1);
        }
        self.build(input, node * 2 + 2, next, center + 1, right);
    }

    /// QuickSelect with a median of three, as `partition_input`.
    fn partition(
        &self,
        input: &mut [usize],
        dimension: usize,
        mut left: usize,
        mut right: usize,
        k: usize,
    ) {
        while left < right {
            let center = (left + right) / 2;
            let pivot;
            {
                let mut left_value = self.coordinate(input[left], dimension);
                let mut center_value = self.coordinate(input[center], dimension);
                let right_value = self.coordinate(input[right], dimension);
                if left_value > center_value {
                    input.swap(left, center);
                    std::mem::swap(&mut left_value, &mut center_value);
                }
                let mut right_value = right_value;
                if left_value > right_value {
                    input.swap(left, right);
                    right_value = left_value;
                }
                if center_value > right_value {
                    input.swap(center, right);
                    center_value = right_value;
                }
                pivot = center_value;
            }
            if right <= left + 2 {
                break;
            }
            let mut i = left;
            let mut j = right - 1;
            input.swap(center, j);
            loop {
                loop {
                    i += 1;
                    if self.coordinate(input[i], dimension) >= pivot {
                        break;
                    }
                }
                loop {
                    j -= 1;
                    if !(self.coordinate(input[j], dimension) > pivot && i < j) {
                        break;
                    }
                }
                if i >= j {
                    break;
                }
                input.swap(i, j);
            }
            input.swap(i, right - 1);
            if k < i {
                right = i - 1;
            } else if k == i {
                break;
            } else {
                left = i + 1;
            }
        }
    }

    /// The nearest point `allowed` lets through; of several as near, the
    /// one visited last (`find_closest_points` with one result).
    fn closest(&self, point: [f64; 2], allowed: &dyn Fn(usize) -> bool) -> usize {
        let mut best = (NONE, f64::MAX);
        if !self.nodes.is_empty() {
            self.visit(0, 0, point, allowed, &mut best);
        }
        best.0
    }

    fn visit(
        &self,
        node: usize,
        dimension: usize,
        point: [f64; 2],
        allowed: &dyn Fn(usize) -> bool,
        best: &mut (usize, f64),
    ) {
        if node >= self.nodes.len() || self.nodes[node] == NONE {
            return;
        }
        let index = self.nodes[node];
        if allowed(index) {
            let distance = (0..2)
                .map(|axis| (point[axis] - self.coordinate(index, axis)).powi(2))
                .sum::<f64>();
            if best.1 >= distance {
                *best = (index, distance);
            }
        }
        let apart = point[dimension] - self.coordinate(index, dimension);
        let (left, right) = if apart * apart < best.1 + EPSILON {
            (true, true)
        } else {
            (apart <= 0.0, apart > 0.0)
        };
        let next = (dimension + 1) % 2;
        if left {
            self.visit(node * 2 + 1, next, point, allowed, best);
        }
        if right {
            self.visit(node * 2 + 2, next, point, allowed, best);
        }
    }
}

/// One end of a segment (`EndPoint`).
#[derive(Clone, Copy)]
struct End {
    pos: [f64; 2],
    chain: usize,
    edge_out: Option<usize>,
    distance_out: f64,
    heap_at: usize,
}

/// MutablePriorityQueue over the ends, least `distance_out` on top.
struct Heap {
    heap: Vec<usize>,
}

impl Heap {
    fn less(ends: &[End], a: usize, b: usize) -> bool {
        ends[a].distance_out < ends[b].distance_out
    }

    fn set(&mut self, ends: &mut [End], at: usize, end: usize) {
        self.heap[at] = end;
        ends[end].heap_at = at;
    }

    fn push(&mut self, ends: &mut [End], end: usize) {
        let at = self.heap.len();
        self.heap.push(end);
        ends[end].heap_at = at;
        self.up(ends, 0, at);
    }

    fn top(&self) -> usize {
        self.heap[0]
    }

    fn pop(&mut self, ends: &mut [End]) {
        ends[self.heap[0]].heap_at = NONE;
        if self.heap.len() > 1 {
            let last = self.heap.pop().unwrap_or(NONE);
            self.set(ends, 0, last);
            let bottom = self.heap.len() - 1;
            self.down(ends, 0, bottom);
        } else {
            self.heap.clear();
        }
    }

    fn remove(&mut self, ends: &mut [End], at: usize) {
        ends[self.heap[at]].heap_at = NONE;
        if at + 1 == self.heap.len() {
            self.heap.pop();
            return;
        }
        let last = self.heap.pop().unwrap_or(NONE);
        self.set(ends, at, last);
        let bottom = self.heap.len() - 1;
        self.down(ends, at, bottom);
        self.up(ends, 0, at);
    }

    fn update(&mut self, ends: &mut [End], at: usize) {
        let end = self.heap[at];
        self.remove(ends, at);
        self.push(ends, end);
    }

    fn up(&mut self, ends: &mut [End], top: usize, bottom: usize) {
        let mut child = bottom;
        loop {
            if child == 0 {
                break;
            }
            let parent = (child - 1) >> 1;
            if parent < top {
                break;
            }
            let (p, c) = (self.heap[parent], self.heap[child]);
            if !Self::less(ends, p, c) {
                self.set(ends, child, p);
                self.set(ends, parent, c);
            }
            child = parent;
        }
    }

    fn down(&mut self, ends: &mut [End], top: usize, bottom: usize) {
        let mut parent = top;
        loop {
            let mut child = (parent << 1) + 1;
            if child > bottom {
                break;
            }
            let second = child + 1;
            if second <= bottom && !Self::less(ends, self.heap[child], self.heap[second]) {
                child = second;
            }
            let (p, c) = (self.heap[parent], self.heap[child]);
            if Self::less(ends, p, c) {
                return;
            }
            self.set(ends, child, p);
            self.set(ends, parent, c);
            parent = child;
        }
    }
}

/// Chains joined, each known by the lowest of their numbers
/// (`EquivalentChains`).
struct Chains {
    same_as: Vec<usize>,
}

impl Chains {
    fn next(&mut self) -> usize {
        self.same_as.push(self.same_as.len());
        self.same_as.len() - 1
    }

    fn of(&mut self, chain: usize) -> usize {
        if chain == 0 {
            return 0;
        }
        let mut last = chain;
        loop {
            let lower = self.same_as[last];
            if lower == last {
                self.same_as[chain] = lower;
                return lower;
            }
            last = lower;
        }
    }

    /// [`Self::of`] without shortening the links on the way, which
    /// changes nothing but how long the next look takes.
    fn find(&self, chain: usize) -> usize {
        let mut last = chain;
        while self.same_as[last] != last {
            last = self.same_as[last];
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

/// The order to visit `points` in (in the reference's units), starting
/// nearest `start` when given; without it, at whichever end of the chain
/// the reference's queue leaves on top.
pub(crate) fn order(points: &[[f64; 2]], start: Option<[f64; 2]>) -> Vec<usize> {
    let count = points.len();
    if count <= 1 {
        return (0..count).collect();
    }
    let at: Vec<[f64; 2]> = points.iter().flat_map(|point| [*point, *point]).collect();
    let tree = KdTree::new(&at);
    let mut ends: Vec<End> = at
        .iter()
        .map(|pos| End {
            pos: *pos,
            chain: 0,
            edge_out: None,
            distance_out: f64::MAX,
            heap_at: NONE,
        })
        .collect();
    let apart = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    let mut chains = Chains { same_as: vec![0] };
    let mut first = None;
    if let Some(start) = start {
        let index = tree.closest(start, &|_| true);
        ends[index].distance_out = 0.0;
        ends[index].chain = chains.next();
        first = Some(index);
    }
    let first_index = first.unwrap_or(NONE);
    for this in 0..ends.len() {
        if Some(this) == first {
            continue;
        }
        let next = tree.closest(ends[this].pos, &|other| {
            other != first_index && (other ^ this) > 1
        });
        if next != NONE {
            ends[this].edge_out = Some(next);
            ends[this].distance_out = apart(ends[next].pos, ends[this].pos);
        }
    }
    let mut queue = Heap {
        heap: Vec::with_capacity(ends.len()),
    };
    for end in 0..ends.len() {
        if Some(end) != first {
            queue.push(&mut ends, end);
        }
    }
    let mut iter = count as isize - 2;
    loop {
        let one = queue.top();
        let Some(other) = ends[one].edge_out else {
            // Nothing left to join to: cannot happen with two or more
            // segments; keep the copies' own order rather than loop.
            return (0..count).collect();
        };
        let mut valid = true;
        let (mut one_chain, mut other_chain) = (0, 0);
        if ends[other].chain > 0 {
            valid = false;
        } else {
            one_chain = chains.of(ends[one ^ 1].chain);
            other_chain = chains.of(ends[other ^ 1].chain);
            if one_chain == other_chain && one_chain != 0 {
                valid = false;
            }
        }
        if valid {
            queue.pop(&mut ends);
            let at = ends[other].heap_at;
            queue.remove(&mut ends, at);
            ends[other].edge_out = Some(one);
            ends[other].distance_out = ends[one].distance_out;
            let chain = match (one_chain, other_chain) {
                (0, 0) => chains.next(),
                (0, chain) | (chain, 0) => chain,
                (a, b) if a == b => a,
                (a, b) => chains.merge(a, b),
            };
            ends[one].chain = chain;
            ends[other].chain = chain;
            if iter == 0 {
                if first.is_none() {
                    let top = queue.top();
                    queue.pop(&mut ends);
                    ends[top].edge_out = None;
                    first = Some(top);
                }
                let last = queue.top();
                ends[last].edge_out = None;
                queue.pop(&mut ends);
                break;
            }
        } else {
            iter += 1;
            ends[one].edge_out = None;
            let this = one;
            let this_chain = chains.find(ends[this ^ 1].chain);
            let next = tree.closest(ends[this].pos, &|other| {
                if (other ^ this) <= 1 || ends[other].chain != 0 {
                    return false;
                }
                let other_chain = chains.find(ends[other ^ 1].chain);
                this_chain != other_chain || this_chain == 0
            });
            if next == NONE {
                return (0..count).collect();
            }
            ends[this].edge_out = Some(next);
            ends[this].distance_out = apart(ends[next].pos, ends[this].pos);
            let at = ends[this].heap_at;
            queue.update(&mut ends, at);
        }
        iter -= 1;
    }
    let mut out = Vec::with_capacity(count);
    let mut point = first;
    while let Some(end) = point {
        if out.len() == count {
            break;
        }
        out.push(end >> 1);
        point = ends[end ^ 1].edge_out;
    }
    if out.len() != count {
        return (0..count).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_in_a_row_go_end_to_end_and_from_the_start_given() {
        let row: Vec<[f64; 2]> = (0..5).map(|i| [i as f64 * 10.0, 0.0]).collect();
        let forwards = order(&row, Some([-100.0, 0.0]));
        assert_eq!(forwards, [0, 1, 2, 3, 4]);
        let backwards = order(&row, Some([100.0, 0.0]));
        assert_eq!(backwards, [4, 3, 2, 1, 0]);
        // Without a start, a chain all the same, from one end or the other.
        let free = order(&row, None);
        assert!(
            free == [0, 1, 2, 3, 4] || free == [4, 3, 2, 1, 0],
            "{free:?}"
        );
        assert_eq!(order(&row[..1], None), [0]);
        assert!(order(&[], None).is_empty());
    }
}
