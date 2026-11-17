//! The brim shared out among the filaments by the wall each stretch of it
//! lies nearest (`brim_filament_source` "nearest_wall"), so that a brim
//! matches what it touches on a print in several filaments.
//!
//! After the reference's BrimFilament.cpp and WallSampleIndex.cpp (the
//! "chameleon brim"): the first layer's walls are sampled every 0.8 mm,
//! each brim line is sampled as finely and each sample goes to the
//! filament its three nearest wall samples weigh for most, by the inverse
//! square of their distance; runs shorter than 2 mm join the run before,
//! and a line is cut into four runs at most.
//!
//! Two differences, on purpose. A line whose samples all go to one
//! filament stays a loop, laid from wherever is nearest as any brim loop
//! is; the reference keeps it a loop only when that filament is its
//! object's own and otherwise lays it as an open line from its first
//! point, which is only how its code happens to fall out. And the brim
//! here is one for the plate, not one per object, so an object with no
//! wall of its own on the first layer (its first layer all support) does
//! not keep its brim in its own filament: its brim goes, like the rest,
//! to the nearest wall there is.

use std::collections::{BTreeMap, HashMap};

use edge_extrusion::{ExtrusionLoop, ExtrusionPath};
use edge_geom::{Coord, Point2, scaled, unscaled};

/// How the reference's vote is weighed (BrimFilament.hpp `BrimVoteParams`
/// as the brim uses it).
const NEAREST: usize = 3;
const TIE_SCORE: f64 = 0.30;
const TIE_DISTANCE_MM: f64 = 0.3;
const SAMPLE_MM: f64 = 0.8;
const SHORTEST_RUN_MM: f64 = 2.0;
const MOST_RUNS: usize = 4;
const CELL_MM: f64 = 2.0;

#[derive(Clone, Copy, Debug)]
struct Sample {
    at: Point2,
    filament: usize,
    object: usize,
}

/// The first layer's walls, sampled along their length for the vote.
#[derive(Debug, Default)]
pub struct WallSamples {
    cells: HashMap<(i64, i64), Vec<Sample>>,
    count: usize,
    /// Each object's first-layer area, which settles a tie between two
    /// filaments for the one whose nearest wall is on the larger object.
    areas: Vec<f64>,
}

fn cell_of(at: Point2) -> (i64, i64) {
    let cell = scaled(CELL_MM) as f64;
    (
        (at.x as f64 / cell).floor() as i64,
        (at.y as f64 / cell).floor() as i64,
    )
}

/// `points` with a point added every `SAMPLE_MM` along each of its edges,
/// every one of its own points kept (WallSampleIndex.cpp `add_polyline`,
/// BrimFilament.cpp `build_chain`).
fn resampled(points: &[Point2]) -> Vec<Point2> {
    let mut chain = Vec::with_capacity(points.len());
    let step = scaled(SAMPLE_MM) as f64;
    for pair in points.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        chain.push(from);
        let (dx, dy) = ((to.x - from.x) as f64, (to.y - from.y) as f64);
        let length = dx.hypot(dy);
        let mut along = step;
        while along < length {
            let share = along / length;
            chain.push(Point2 {
                x: (from.x as f64 + dx * share).round() as Coord,
                y: (from.y as f64 + dy * share).round() as Coord,
            });
            along += step;
        }
    }
    if let Some(last) = points.last() {
        chain.push(*last);
    }
    chain
}

impl WallSamples {
    /// With the first-layer area of each object, by its number.
    pub fn new(areas: Vec<f64>) -> Self {
        Self {
            areas,
            ..Self::default()
        }
    }

    /// A wall of `object`'s, closed, laid in `filament`.
    pub fn add_wall(&mut self, points: &[Point2], filament: usize, object: usize) {
        let Some(first) = points.first() else {
            return;
        };
        let mut closed = points.to_vec();
        closed.push(*first);
        for at in resampled(&closed) {
            self.cells.entry(cell_of(at)).or_default().push(Sample {
                at,
                filament,
                object,
            });
            self.count += 1;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The `NEAREST` samples nearest `at`, nearest first, ties by filament
    /// and then object (WallSampleIndex.cpp `knn`): rings of cells round
    /// `at`'s, until no cell further out can hold a nearer one.
    fn nearest(&self, at: Point2) -> Vec<(Sample, f64)> {
        let mut found: Vec<(Sample, f64)> = Vec::new();
        if self.count == 0 {
            return found;
        }
        let cell = scaled(CELL_MM) as f64;
        let (cx, cy) = cell_of(at);
        let (px, py) = (at.x as f64, at.y as f64);
        let take = |x: i64, y: i64, found: &mut Vec<(Sample, f64)>| {
            if let Some(samples) = self.cells.get(&(x, y)) {
                for sample in samples {
                    let (dx, dy) = (sample.at.x as f64 - px, sample.at.y as f64 - py);
                    found.push((*sample, dx * dx + dy * dy));
                }
            }
        };
        let order = |a: &(Sample, f64), b: &(Sample, f64)| {
            a.1.total_cmp(&b.1)
                .then(a.0.filament.cmp(&b.0.filament))
                .then(a.0.object.cmp(&b.0.object))
        };
        take(cx, cy, &mut found);
        let wanted = NEAREST.min(self.count);
        let mut radius = 0;
        loop {
            if found.len() >= wanted {
                found.sort_by(order);
                let kth = found[wanted - 1].1;
                let (x0, x1) = ((cx - radius) as f64 * cell, (cx + radius + 1) as f64 * cell);
                let (y0, y1) = ((cy - radius) as f64 * cell, (cy + radius + 1) as f64 * cell);
                let next = (px - x0).min(x1 - px).min(py - y0).min(y1 - py);
                if found.len() >= self.count || next * next > kth {
                    break;
                }
            }
            radius += 1;
            for x in cx - radius..=cx + radius {
                take(x, cy - radius, &mut found);
                take(x, cy + radius, &mut found);
            }
            for y in cy - radius + 1..cy + radius {
                take(cx - radius, y, &mut found);
                take(cx + radius, y, &mut found);
            }
        }
        found.sort_by(order);
        found.truncate(NEAREST);
        found
    }

    /// The filament the brim at `at` is laid in (BrimFilament.cpp
    /// `brim_vote`): the most weight by the inverse square of distance,
    /// and where the two heaviest come near each other (within 30 % of
    /// weight, or their nearest walls within 0.3 mm of the same
    /// distance), the one whose nearest wall is on the larger object, and
    /// then the lower filament. None when there are no walls.
    pub fn vote(&self, at: Point2) -> Option<usize> {
        let nearest = self.nearest(at);
        let floor = {
            let hundredth = scaled(0.01) as f64;
            hundredth * hundredth
        };
        // By filament: weight, nearest distance squared, and the object
        // of the nearest.
        let mut by: BTreeMap<usize, (f64, f64, usize)> = BTreeMap::new();
        for (sample, distance) in &nearest {
            let entry = by
                .entry(sample.filament)
                .or_insert((0.0, f64::INFINITY, sample.object));
            entry.0 += 1.0 / distance.max(floor);
            if *distance < entry.1 {
                (entry.1, entry.2) = (*distance, sample.object);
            }
        }
        let mut heaviest: Option<(usize, (f64, f64, usize))> = None;
        for (filament, weighed) in &by {
            if heaviest.is_none_or(|(_, best)| weighed.0 > best.0) {
                heaviest = Some((*filament, *weighed));
            }
        }
        let (winner, won) = heaviest?;
        let mut next: Option<(usize, (f64, f64, usize))> = None;
        for (filament, weighed) in by.iter().filter(|(filament, _)| **filament != winner) {
            if next.is_none_or(|(_, best)| weighed.0 > best.0) {
                next = Some((*filament, *weighed));
            }
        }
        let Some((runner, ran)) = next else {
            return Some(winner);
        };
        let tie = ran.0 >= won.0 * (1.0 - TIE_SCORE)
            || (won.1.sqrt() - ran.1.sqrt()).abs() < scaled(TIE_DISTANCE_MM) as f64;
        if !tie {
            return Some(winner);
        }
        let area = |object: usize| self.areas.get(object).copied().unwrap_or(0.0);
        let (won_area, ran_area) = (area(won.2), area(ran.2));
        Some(if won_area > ran_area {
            winner
        } else if ran_area > won_area {
            runner
        } else {
            winner.min(runner)
        })
    }
}

/// One stretch of a brim line in one filament.
struct Run {
    filament: usize,
    points: Vec<Point2>,
}

fn length_mm(points: &[Point2]) -> f64 {
    points
        .windows(2)
        .map(|pair| {
            unscaled(1) * ((pair[1].x - pair[0].x) as f64).hypot((pair[1].y - pair[0].y) as f64)
        })
        .sum()
}

/// Runs shorter than the shortest kept go into the run before them, the
/// first into the one after (BrimFilament.cpp `absorb_short_runs`).
fn absorb_short(runs: &mut Vec<Run>) {
    while runs.len() > 1 {
        let Some(short) = runs
            .iter()
            .position(|run| length_mm(&run.points) < SHORTEST_RUN_MM)
        else {
            return;
        };
        let taken = runs.remove(short);
        if short == 0 {
            runs[0].points.splice(0..0, taken.points);
        } else {
            runs[short - 1].points.extend(taken.points);
        }
    }
}

/// While there are too many runs, the shortest goes into the longer of
/// the runs either side of it, the one before where they are as long
/// (BrimFilament.cpp `guard_max_runs`).
fn at_most(runs: &mut Vec<Run>) {
    while runs.len() > MOST_RUNS {
        let lengths: Vec<f64> = runs.iter().map(|run| length_mm(&run.points)).collect();
        let mut short = 0;
        for (at, length) in lengths.iter().enumerate() {
            if *length < lengths[short] {
                short = at;
            }
        }
        let into_next = match (short > 0, short + 1 < runs.len()) {
            (true, true) => lengths[short + 1] > lengths[short - 1],
            (_, has_next) => has_next,
        };
        let taken = runs.remove(short);
        if into_next {
            runs[short].points.splice(0..0, taken.points);
        } else {
            runs[short - 1].points.extend(taken.points);
        }
    }
}

/// A closed brim line cut where the vote changes, the runs sharing their
/// ends (BrimFilament.cpp `split_polyline_core`, as the brim calls it).
fn runs_of(polygon: &[Point2], samples: &WallSamples) -> Option<Vec<Run>> {
    let first = *polygon.first()?;
    let mut closed = polygon.to_vec();
    closed.push(first);
    let chain = resampled(&closed);
    let mut runs: Vec<Run> = Vec::new();
    for at in chain {
        let filament = samples.vote(at)?;
        match runs.last_mut() {
            Some(run) if run.filament == filament => run.points.push(at),
            _ => runs.push(Run {
                filament,
                points: vec![at],
            }),
        }
    }
    absorb_short(&mut runs);
    at_most(&mut runs);
    for at in 1..runs.len() {
        let end = *runs[at - 1].points.last()?;
        runs[at].points.insert(0, end);
    }
    Some(runs)
}

/// One piece of the brim and the filament that lays it.
#[derive(Clone, Debug)]
pub enum BrimPiece {
    /// A whole loop.
    Whole(ExtrusionLoop),
    /// A stretch of one, laid as an open line.
    Stretch(ExtrusionPath),
}

/// The brim's loops shared out among the filaments by the nearest wall,
/// in their order, each with its filament. With no walls to go by, None.
pub fn share_brim(
    brim: &[ExtrusionLoop],
    samples: &WallSamples,
) -> Option<Vec<(usize, BrimPiece)>> {
    if samples.is_empty() {
        return None;
    }
    let mut pieces = Vec::with_capacity(brim.len());
    for line in brim {
        let Some(runs) = runs_of(&line.polygon.points, samples) else {
            continue;
        };
        if let [run] = runs.as_slice() {
            pieces.push((run.filament, BrimPiece::Whole(line.clone())));
            continue;
        }
        for run in runs {
            pieces.push((
                run.filament,
                BrimPiece::Stretch(ExtrusionPath {
                    points: run.points,
                    role: line.role,
                    flow: line.flow,
                    in_turn: false,
                    part: 0,
                }),
            ));
        }
    }
    Some(pieces)
}

#[cfg(test)]
mod tests {
    use edge_extrusion::{Flow, Role};
    use edge_geom::Polygon;

    use super::*;

    fn square(x: f64, y: f64, side: f64) -> Vec<Point2> {
        [(x, y), (x + side, y), (x + side, y + side), (x, y + side)]
            .map(|(x, y)| Point2::from_mm(x, y))
            .to_vec()
    }

    fn brim_loop(points: Vec<Point2>) -> ExtrusionLoop {
        ExtrusionLoop {
            polygon: Polygon { points },
            role: Role::Brim,
            is_hole: false,
            depth: 0,
            flow: Flow::new(0.5, 0.2, 0.4).unwrap(),
            widths: Vec::new(),
            clockwise: false,
            circle_compensation: false,
        }
    }

    #[test]
    fn the_nearest_walls_say_which_filament() {
        // Two parts side by side, one in each filament.
        let mut samples = WallSamples::new(vec![100.0, 100.0]);
        samples.add_wall(&square(0.0, 0.0, 10.0), 0, 0);
        samples.add_wall(&square(10.0, 0.0, 10.0), 1, 1);
        assert_eq!(samples.vote(Point2::from_mm(-1.0, 5.0)), Some(0));
        assert_eq!(samples.vote(Point2::from_mm(21.0, 5.0)), Some(1));
        // Midway, the two weigh the same: the lower filament.
        assert_eq!(samples.vote(Point2::from_mm(10.0, -1.0)), Some(0));
        // Or the larger object's, where they tie.
        let mut larger = WallSamples::new(vec![100.0, 200.0]);
        larger.add_wall(&square(0.0, 0.0, 10.0), 0, 0);
        larger.add_wall(&square(10.0, 0.0, 10.0), 1, 1);
        assert_eq!(larger.vote(Point2::from_mm(10.0, -1.0)), Some(1));
        assert_eq!(WallSamples::default().vote(Point2::from_mm(0.0, 0.0)), None);
    }

    #[test]
    fn a_brim_round_two_filaments_is_cut_between_them() {
        let mut samples = WallSamples::new(vec![100.0, 100.0]);
        samples.add_wall(&square(0.0, 0.0, 10.0), 0, 0);
        samples.add_wall(&square(10.0, 0.0, 10.0), 1, 1);
        // A brim loop round both, and one round a part of one filament
        // standing alone further off.
        let round_both = brim_loop(square(-0.5, -0.5, 21.0));
        let mut far = WallSamples::new(vec![100.0, 100.0]);
        far.add_wall(&square(0.0, 0.0, 10.0), 0, 0);
        let pieces = share_brim(std::slice::from_ref(&round_both), &samples).unwrap();
        let stretches: Vec<(usize, &ExtrusionPath)> = pieces
            .iter()
            .filter_map(|(filament, piece)| match piece {
                BrimPiece::Stretch(path) => Some((*filament, path)),
                BrimPiece::Whole(_) => None,
            })
            .collect();
        assert_eq!(stretches.len(), pieces.len());
        assert!((2..=MOST_RUNS).contains(&stretches.len()));
        // Each filament gets about half the loop's 84 mm, the stretches
        // meet end to end, and none is shorter than the shortest kept.
        let mut by = [0.0; 2];
        for (filament, path) in &stretches {
            by[*filament] += length_mm(&path.points);
            assert!(length_mm(&path.points) >= SHORTEST_RUN_MM);
            assert_eq!(path.role, Role::Brim);
        }
        assert!(
            (by[0] - 42.0).abs() < 2.5 && (by[1] - 42.0).abs() < 2.5,
            "{by:?}"
        );
        for pair in stretches.windows(2) {
            assert_eq!(pair[0].1.points.last(), pair[1].1.points.first());
        }
        // All of a loop in one filament stays a whole loop.
        let alone = share_brim(&[round_both], &far).unwrap();
        assert!(matches!(alone.as_slice(), [(0, BrimPiece::Whole(_))]));
        assert!(share_brim(&[], &WallSamples::default()).is_none());
    }

    #[test]
    fn short_runs_join_the_run_before_and_at_most_four_are_kept() {
        let line = |from: f64, to: f64| vec![Point2::from_mm(from, 0.0), Point2::from_mm(to, 0.0)];
        let mut runs: Vec<Run> = [(0, 0.0, 5.0), (1, 5.0, 6.0), (0, 6.0, 10.0)]
            .map(|(filament, from, to)| Run {
                filament,
                points: line(from, to),
            })
            .into();
        absorb_short(&mut runs);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].points.len(), 4);
        let mut many: Vec<Run> = (0..6)
            .map(|at| Run {
                filament: at % 2,
                points: line(at as f64 * 3.0, at as f64 * 3.0 + 3.0 + at as f64 * 0.1),
            })
            .collect();
        at_most(&mut many);
        assert_eq!(many.len(), MOST_RUNS);
    }
}
