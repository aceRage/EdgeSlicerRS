//! Where a branch may and may not be, for each radius it can have and each
//! layer (TreeModelVolumes.cpp): the part grown by the gap and the radius
//! (collision), the same with holes a branch cannot pass closed
//! (hole-free), everywhere from which a branch could not get down to the
//! bed or onto the part at its lean (avoidance, in three speeds), the
//! flat tops a branch may stand on (placeable), and the walls an influence
//! area must not jump between two layers (wall restriction).
//!
//! As in the reference everything is worked out up front for the radii
//! the trees will ask for, up to the highest layer each is needed on
//! (`precalculate`); after that the volumes are only read, so the
//! propagation can run in parallel without locks. A radius asked for that
//! was not foreseen gets the next larger one worked out, which keeps a
//! branch further from the part, never nearer.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use edge_clip::Join;
use edge_geom::{Coord, ExPolygon, scaled};
use rayon::prelude::*;

use super::config::{EPSILON, TreeConfig};
use super::geometry::{MITER, Polys, diff, intersection, offset, round, simplify, union};
use crate::SupportError;

/// TreeModelVolumes.hpp `SUPPORT_TREE_EXPONENTIAL_FACTOR`, `_THRESHOLD` and
/// `SUPPORT_TREE_COLLISION_RESOLUTION`.
const EXPONENTIAL_FACTOR: f64 = 1.5;
const EXPONENTIAL_THRESHOLD: Coord = 1_500_000;
const COLLISION_RESOLUTION: Coord = 500_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Avoidance {
    /// Reachable at the usual lean, keeping clear of holes.
    Slow,
    /// Reachable at the greatest lean, keeping clear of holes.
    FastSafe,
    /// Reachable at the greatest lean.
    Fast,
}

/// One kind of volume for one radius, layer by layer from the bed.
type Column = Vec<Polys>;

pub(crate) struct Volumes {
    outlines: Vec<Polys>,
    radius_0: Coord,
    ignorable: Vec<Coord>,
    min_xy_dist: Coord,
    min_xy_dist_delta: Coord,
    increase_until_radius: Coord,
    max_move: Coord,
    max_move_slow: Coord,
    resolution: Coord,
    z_bottom: usize,
    z_top: usize,
    rests_on_model: bool,
    collision: BTreeMap<Coord, Column>,
    holefree: BTreeMap<Coord, Column>,
    avoidance: BTreeMap<(Coord, Avoidance, bool), Column>,
    placeable: BTreeMap<Coord, Column>,
    walls: BTreeMap<(Coord, bool), Column>,
    /// Requests for a radius or layer that was not worked out.
    pub misses: AtomicUsize,
    /// Where branches may not go, by layer: the support blockers
    /// (TreeModelVolumes.cpp `m_anti_overhang`).
    pub anti_overhang: Vec<Polys>,
}

impl Volumes {
    pub fn new(slices: &[&[ExPolygon]], config: &TreeConfig) -> Result<Self, SupportError> {
        // TreeModelVolumes(): each layer's outline, thinned to the print's
        // resolution.
        let outlines = slices
            .par_iter()
            .map(|layer| simplify(&union(layer)?, config.resolution))
            .collect::<Result<Vec<Polys>, SupportError>>()?;
        Ok(Self {
            outlines,
            radius_0: config.radius(0, 0.0),
            ignorable: Vec::new(),
            min_xy_dist: config.xy_min_distance,
            min_xy_dist_delta: config.xy_distance - config.xy_min_distance,
            increase_until_radius: config.increase_radius_until_radius,
            max_move: (config.max_move - 2).max(0),
            max_move_slow: (config.max_move_slow - 2).max(0),
            resolution: config.resolution.max(1),
            z_bottom: config.z_distance_bottom_layers,
            z_top: config.z_distance_top_layers,
            rests_on_model: config.rests_on_model,
            collision: BTreeMap::new(),
            holefree: BTreeMap::new(),
            avoidance: BTreeMap::new(),
            placeable: BTreeMap::new(),
            walls: BTreeMap::new(),
            misses: AtomicUsize::new(0),
            anti_overhang: Vec::new(),
        })
    }

    /// The radii collisions and avoidances are worked out for: the tip's,
    /// then steps to 1.5 mm, then half as much again each time, skipping
    /// those no branch has (TreeModelVolumes::ceilRadius).
    pub fn ceil_radius(&self, radius: Coord) -> Coord {
        if radius == 0 {
            return 0;
        }
        let mut out = self.radius_0;
        if radius > self.radius_0 {
            let ignore = |r: Coord| self.ignorable.binary_search(&r).is_ok();
            let initial_delta = EXPONENTIAL_THRESHOLD - self.radius_0;
            if initial_delta > COLLISION_RESOLUTION {
                let steps = super::config::round_up_divide(initial_delta, EXPONENTIAL_THRESHOLD);
                let step_size = initial_delta / steps;
                out += step_size;
                for _ in 0..steps {
                    if out >= radius && !ignore(out) {
                        return out;
                    }
                    out += step_size;
                }
            } else {
                out += COLLISION_RESOLUTION;
            }
            while out < radius || ignore(out) {
                out = (out as f64 * EXPONENTIAL_FACTOR) as Coord;
            }
        }
        out
    }

    /// The radius the volumes of a branch of `radius` are kept under:
    /// with the smaller gap beside overhangs, or with the full one
    /// (`ceilRadius(radius, min_xy_dist)`).
    pub fn ceil_radius_xy(&self, radius: Coord, min_xy_dist: bool) -> Coord {
        if min_xy_dist {
            self.ceil_radius(radius)
        } else if radius > 0 {
            self.ceil_radius(radius + self.min_xy_dist_delta)
        } else {
            self.min_xy_dist_delta
        }
    }

    /// `getRadiusNextCeil`.
    pub fn radius_next_ceil(&self, radius: Coord, min_xy_dist: bool) -> Coord {
        if min_xy_dist {
            self.ceil_radius(radius)
        } else {
            self.ceil_radius(radius + self.min_xy_dist_delta) - self.min_xy_dist_delta
        }
    }

    /// Works out every volume the trees can ask for up to `max_layer`
    /// (TreeModelVolumes::precalculate).
    pub fn precalculate(
        &mut self,
        config: &TreeConfig,
        max_layer: usize,
    ) -> Result<(), SupportError> {
        // Radii a tip may have, and those in between that no branch has.
        let mut tip_radii = Vec::new();
        for dtt in 0..=config.tip_layers {
            tip_radii.push(self.ceil_radius(config.radius(dtt, 0.0)));
            tip_radii.push(self.ceil_radius(config.radius(dtt, 0.0) + self.min_xy_dist_delta));
        }
        tip_radii.sort_unstable();
        tip_radii.dedup();
        let mut eval = self.radius_0;
        while eval <= config.branch_radius {
            if tip_radii.binary_search(&eval).is_err() {
                self.ignorable.push(eval);
            }
            eval = self.ceil_radius(eval + 1);
        }
        // Up to which layer each radius is needed: a branch reaches a
        // radius some layers under its tip, so the higher the tip ...
        let mut until: BTreeMap<Coord, usize> = BTreeMap::new();
        for dtt in 0..=max_layer {
            let layer = max_layer - dtt;
            for radius in [
                self.ceil_radius(config.radius(dtt, 0.0) + self.min_xy_dist_delta),
                self.ceil_radius(config.radius(dtt, 0.0)),
                self.ceil_radius(
                    config.recommended_min_radius(layer as i64) + self.min_xy_dist_delta,
                ),
            ] {
                until.entry(radius).or_insert(layer);
            }
        }
        let avoidance_radii: Vec<(Coord, usize)> = until.iter().map(|(r, l)| (*r, *l)).collect();
        until.insert(
            self.ceil_radius(self.increase_until_radius + self.min_xy_dist_delta),
            max_layer,
        );
        until.insert(0, max_layer);
        if self.min_xy_dist_delta != 0 {
            until.insert(self.min_xy_dist_delta, max_layer);
        }
        let max_layer = max_layer.min(self.outlines.len().saturating_sub(1));
        let collision_radii: Vec<(Coord, usize)> = until
            .iter()
            .map(|(r, l)| (*r, (*l).min(max_layer)))
            .collect();

        // Collisions, and the places a branch may stand with radius 0.
        let columns = collision_radii
            .par_iter()
            .map(|&(radius, top)| self.collision_column(radius, top))
            .collect::<Result<Vec<_>, SupportError>>()?;
        for (&(radius, _), (column, placeable)) in collision_radii.iter().zip(columns) {
            self.collision.insert(radius, column);
            if let Some(placeable) = placeable {
                self.placeable.insert(0, placeable);
            }
        }
        // Collisions with holes closed, for radii below the one from which
        // a branch is wide enough not to fit any hole it would have to.
        let hole_limit = self.increase_until_radius + self.min_xy_dist_delta;
        let holefree_radii: Vec<(Coord, usize)> = avoidance_radii
            .iter()
            .filter(|(radius, _)| *radius < hole_limit)
            .map(|&(radius, top)| (radius, top.min(max_layer)))
            .collect();
        let columns = holefree_radii
            .par_iter()
            .map(|&(radius, top)| {
                let grown = self.ceil_radius_xy(self.increase_until_radius, false);
                (0..=top)
                    .into_par_iter()
                    .map(|layer| {
                        let source = self.collision(self.increase_until_radius, layer, false);
                        simplify(
                            &offset(
                                &union(source)?,
                                5 - (grown - radius),
                                round(self.resolution),
                            )?,
                            self.resolution,
                        )
                    })
                    .collect::<Result<Column, SupportError>>()
            })
            .collect::<Result<Vec<_>, SupportError>>()?;
        for (&(radius, _), column) in holefree_radii.iter().zip(columns) {
            self.holefree.insert(radius, column);
        }

        let avoidance_radii: Vec<(Coord, usize)> = avoidance_radii
            .into_iter()
            .map(|(radius, top)| (radius, top.min(max_layer)))
            .collect();
        if self.rests_on_model {
            // Larger radii stand on less: once a layer has nowhere for one
            // radius, it has nowhere for the larger ones either.
            let mut radii: Vec<(Coord, usize)> = avoidance_radii
                .iter()
                .copied()
                .filter(|(radius, _)| *radius != 0)
                .collect();
            radii.sort_unstable();
            let top = radii.iter().map(|(_, top)| *top).max().unwrap_or(0);
            let per_layer: Vec<Vec<Polys>> = (0..=top.min(max_layer))
                .into_par_iter()
                .map(|layer| -> Result<Vec<Polys>, SupportError> {
                    let mut out = Vec::with_capacity(radii.len());
                    let source = if layer == 0 {
                        &[][..]
                    } else {
                        self.placeable(0, layer)
                    };
                    for &(radius, until) in &radii {
                        if layer > until
                            || source.is_empty()
                            || out.last().is_some_and(Vec::is_empty)
                        {
                            out.push(Vec::new());
                            continue;
                        }
                        let shrink = -(radius + self.min_xy_dist + self.min_xy_dist_delta);
                        out.push(offset(source, shrink, MITER)?);
                    }
                    Ok(out)
                })
                .collect::<Result<_, _>>()?;
            for (index, &(radius, until)) in radii.iter().enumerate() {
                let column: Column = per_layer
                    .iter()
                    .take(until.min(max_layer) + 1)
                    .map(|layer| layer[index].clone())
                    .collect();
                self.placeable.insert(radius, column);
            }
        }
        // Avoidances, each a serial pass up the layers, run side by side.
        let mut tasks = Vec::new();
        for &(radius, top) in &avoidance_radii {
            for to_model in [false, true] {
                if to_model && !self.rests_on_model {
                    continue;
                }
                for kind in [Avoidance::Slow, Avoidance::FastSafe, Avoidance::Fast] {
                    if kind == Avoidance::FastSafe && radius >= hole_limit {
                        continue;
                    }
                    tasks.push((radius, top, kind, to_model));
                }
            }
        }
        let wall_tasks: Vec<(Coord, usize, bool)> = avoidance_radii
            .iter()
            .flat_map(|&(radius, top)| {
                let both = self.min_xy_dist_delta > 0;
                [(radius, top, false)]
                    .into_iter()
                    .chain(both.then_some((radius, top, true)))
            })
            .collect();
        let (avoidances, walls) = rayon::join(
            || {
                tasks
                    .par_iter()
                    .map(|&(radius, top, kind, to_model)| {
                        self.avoidance_column(radius, top, kind, to_model)
                    })
                    .collect::<Result<Vec<_>, SupportError>>()
            },
            || {
                wall_tasks
                    .par_iter()
                    .map(|&(radius, top, min)| {
                        (0..=top)
                            .into_par_iter()
                            .map(|layer| {
                                if layer == 0 {
                                    return Ok(Vec::new());
                                }
                                simplify(
                                    &intersection(
                                        self.collision(0, layer, min),
                                        self.collision(radius, layer - 1, true),
                                    )?,
                                    self.resolution,
                                )
                            })
                            .collect::<Result<Column, SupportError>>()
                    })
                    .collect::<Result<Vec<_>, SupportError>>()
            },
        );
        for (&(radius, _, kind, to_model), column) in tasks.iter().zip(avoidances?) {
            if let Some(column) = column {
                self.avoidance.insert((radius, kind, to_model), column);
            }
        }
        for (&(radius, _, min), column) in wall_tasks.iter().zip(walls?) {
            self.walls.insert((radius, min), column);
        }
        Ok(())
    }

    /// The collision of one radius on every layer up to `top`, and with
    /// radius 0 the places a branch may stand (calculateCollision).
    fn collision_column(
        &self,
        radius: Coord,
        top: usize,
    ) -> Result<(Column, Option<Column>), SupportError> {
        let count = self.outlines.len();
        let low = 0;
        let high = (top + self.z_top + 1).min(count);
        let grow = radius + self.min_xy_dist;
        let grown: Vec<Polys> = (low..high)
            .into_par_iter()
            .map(|layer| {
                if grow == 0 {
                    union(&self.outlines[layer])
                } else {
                    offset(&self.outlines[layer], grow, MITER)
                }
            })
            .collect::<Result<_, SupportError>>()?;
        let column = (0..=top)
            .into_par_iter()
            .map(|layer| {
                let mut all: Polys = Vec::new();
                for below in &grown[layer.saturating_sub(self.z_bottom)..=layer] {
                    all.extend(below.iter().cloned());
                }
                for above in 1..=self.z_top {
                    let at = layer + above;
                    if at >= count {
                        break;
                    }
                    // Less of the gap beside the layers above, so that an
                    // overhang thinner than the gap is still held up, and
                    // half a layer's worth more with a gap of one layer, so
                    // that nothing touches on the diagonal.
                    let shift = if self.z_top == 1 { 0.5 } else { 0.0 };
                    let range = (self.min_xy_dist as f64
                        - (above as f64 - shift) * self.min_xy_dist as f64 / self.z_top as f64)
                        as Coord;
                    all.extend(offset(&self.outlines[at], radius + range, MITER)?);
                }
                // No branch through a support blocker (calculateCollision:
                // `anti_overhang` grown by the radius).
                if let Some(anti) = self
                    .anti_overhang
                    .get(layer)
                    .filter(|anti| !anti.is_empty())
                {
                    all.extend(offset(&union(anti)?, radius, MITER)?);
                }
                simplify(&union(&all)?, self.resolution)
            })
            .collect::<Result<Column, SupportError>>()?;
        let placeable = if self.rests_on_model && radius == 0 {
            Some(
                (0..=top)
                    .into_par_iter()
                    .map(|layer| {
                        if layer < self.z_bottom + 1 {
                            return Ok(Vec::new());
                        }
                        let below = layer - self.z_bottom - 1;
                        let surface = offset(&self.outlines[below], self.min_xy_dist, Join::MITER)?;
                        // Nor does one stand on a blocker.
                        let taken = match self
                            .anti_overhang
                            .get(below)
                            .filter(|anti| !anti.is_empty())
                        {
                            Some(anti) => union(&[grown[layer].clone(), anti.clone()].concat())?,
                            None => grown[layer].clone(),
                        };
                        simplify(&diff(&surface, &taken)?, self.resolution)
                    })
                    .collect::<Result<Column, SupportError>>()?,
            )
        } else {
            None
        };
        Ok((column, placeable))
    }

    /// One avoidance, from the bed up (calculateAvoidance).
    fn avoidance_column(
        &self,
        radius: Coord,
        top: usize,
        kind: Avoidance,
        to_model: bool,
    ) -> Result<Option<Column>, SupportError> {
        let hole_limit = self.increase_until_radius + self.min_xy_dist_delta;
        let holefree = kind != Avoidance::Fast && radius < hole_limit;
        let max_move = if kind == Avoidance::Slow {
            self.max_move_slow
        } else {
            self.max_move
        } as f64;
        // Steps short enough that shrinking the layer below and adding this
        // layer's collision cannot open a gap a branch could tunnel through.
        let mut step = 1.9 * radius.max(self.min_xy_dist) as f64;
        if step < EPSILON {
            return Ok(None);
        }
        let mut steps = ((max_move as Coord + step as Coord - 1) / (step as Coord).max(1)).max(1);
        let mut last = max_move - (steps - 1) as f64 * step;
        if last < scaled(0.05) as f64 && steps > 1 {
            steps -= 1;
            step = max_move / steps as f64;
            last = step;
        }
        let mut column: Column = Vec::with_capacity(top + 1);
        let mut latest = self.collision(radius, 0, true).to_vec();
        column.push(latest.clone());
        for layer in 1..=top {
            let current = if holefree {
                self.holefree_at(radius, layer)
            } else {
                self.collision(radius, layer, true)
            };
            for index in 0..steps {
                let by = if index + 1 == steps { last } else { step };
                let shrunk = offset(&latest, -(by as Coord), round(self.resolution))?;
                let mut all = current.to_vec();
                all.extend(shrunk);
                latest = union(&all)?;
            }
            if to_model {
                let placeable = self.placeable(radius, layer);
                if !placeable.is_empty() {
                    latest = diff(&latest, placeable)?;
                }
            }
            latest = simplify(&latest, self.resolution)?;
            column.push(latest.clone());
        }
        Ok(Some(column))
    }

    fn miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    /// The column for the smallest key no smaller than `radius` that
    /// reaches `layer`, or failing that the largest that does.
    fn find<'a, K: Ord + Copy>(
        &self,
        map: &'a BTreeMap<K, Column>,
        key: K,
        layer: usize,
        same: impl Fn(&K) -> bool,
    ) -> Option<&'a Polys> {
        if let Some(column) = map.get(&key)
            && let Some(polys) = column.get(layer)
        {
            return Some(polys);
        }
        self.miss();
        map.range(key..)
            .filter(|(k, _)| same(k))
            .find_map(|(_, column)| column.get(layer))
            .or_else(|| {
                map.range(..key)
                    .rev()
                    .filter(|(k, _)| same(k))
                    .find_map(|(_, column)| column.get(layer))
            })
    }

    /// Where the centre of a branch of `radius` may not be on a layer
    /// (getCollision).
    pub fn collision(&self, radius: Coord, layer: usize, min_xy_dist: bool) -> &[ExPolygon] {
        let key = self.ceil_radius_xy(radius, min_xy_dist);
        self.find(&self.collision, key, layer, |_| true)
            .map_or(&[], Vec::as_slice)
    }

    fn holefree_at(&self, radius: Coord, layer: usize) -> &[ExPolygon] {
        match self
            .holefree
            .get(&radius)
            .and_then(|column| column.get(layer))
        {
            Some(polys) => polys,
            None => {
                self.miss();
                self.collision(radius, layer, true)
            }
        }
    }

    /// Where the centre of a branch could not get down from to the bed,
    /// or with `to_model` onto the part either (getAvoidance).
    pub fn avoidance(
        &self,
        radius: Coord,
        layer: usize,
        mut kind: Avoidance,
        to_model: bool,
        min_xy_dist: bool,
    ) -> &[ExPolygon] {
        if layer == 0 {
            return self.collision(radius, 0, min_xy_dist);
        }
        let key = self.ceil_radius_xy(radius, min_xy_dist);
        if kind == Avoidance::FastSafe && key >= self.increase_until_radius + self.min_xy_dist_delta
        {
            kind = Avoidance::Fast;
        }
        let to_model = to_model && self.rests_on_model;
        self.find(&self.avoidance, (key, kind, to_model), layer, |k| {
            k.1 == kind && k.2 == to_model
        })
        .map_or(&[], Vec::as_slice)
    }

    /// Flat tops of the part a branch of `radius` may stand on
    /// (getPlaceableAreas).
    pub fn placeable(&self, radius: Coord, layer: usize) -> &[ExPolygon] {
        let key = self.ceil_radius(radius);
        match self
            .placeable
            .get(&key)
            .and_then(|column| column.get(layer))
        {
            Some(polys) => polys,
            None => {
                if self.rests_on_model {
                    self.miss();
                }
                // Asking for more than was worked out: a larger radius
                // stands on less.
                self.placeable
                    .range(key..)
                    .find_map(|(_, column)| column.get(layer))
                    .map_or(&[], Vec::as_slice)
            }
        }
    }

    /// The part's walls between a layer and the one below it, which an
    /// influence area must not be grown across (getWallRestriction).
    pub fn wall_restriction(&self, radius: Coord, layer: usize, min_xy_dist: bool) -> &[ExPolygon] {
        if layer == 0 {
            return self.collision(radius, 0, min_xy_dist);
        }
        let min = min_xy_dist && self.min_xy_dist_delta > 0;
        let key = self.ceil_radius(radius);
        match self
            .walls
            .get(&(key, min))
            .and_then(|column| column.get(layer))
        {
            Some(polys) => polys,
            None => {
                self.miss();
                self.walls
                    .range((key, min)..)
                    .filter(|(k, _)| k.1 == min)
                    .find_map(|(_, column)| column.get(layer))
                    .map_or(&[], Vec::as_slice)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::geometry::{contains, rectangle};
    use crate::{Job, SupportSettings};
    use edge_geom::Point2;

    fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> ExPolygon {
        rectangle(scaled(x0), scaled(y0), scaled(x1), scaled(y1))
    }

    #[test]
    fn radii_are_rounded_up_to_a_ladder() {
        let settings = SupportSettings::default();
        let job = Job {
            slices: &[],
            first_layer_height: 0.2,
            layer_height: 0.2,
            settings: &settings,
            gap_above: 1,
            gap_below: 1,
            bed: None,
            annotations: &crate::NO_ANNOTATIONS,
        };
        let config = TreeConfig::new(&job).unwrap();
        let volumes = Volumes::new(&[], &config).unwrap();
        // From the tip's 0.4 mm to 1.5 mm in one step, then that step
        // again and half as much again each time after.
        let ladder: Vec<Coord> = [0.1, 0.4, 0.5, 1.5, 1.6, 2.7, 4.0]
            .iter()
            .map(|r| volumes.ceil_radius(scaled(*r)))
            .collect();
        assert_eq!(
            ladder,
            [0.4, 0.4, 1.5, 1.5, 2.6, 3.9, 5.85].map(scaled).to_vec()
        );
        assert_eq!(volumes.ceil_radius(0), 0);
    }

    #[test]
    fn a_branch_keeps_clear_of_the_part_and_of_what_it_cannot_get_down_from() {
        // A block 10 mm square standing 40 layers tall, and over it from
        // layer 30 a roof reaching 20 mm out to one side.
        let block = vec![square(0.0, 0.0, 10.0, 10.0)];
        let roof = vec![square(0.0, 0.0, 30.0, 10.0)];
        let layers: Vec<Vec<ExPolygon>> = (0..40)
            .map(|layer| {
                if layer < 30 {
                    block.clone()
                } else {
                    roof.clone()
                }
            })
            .collect();
        let slices: Vec<&[ExPolygon]> = layers.iter().map(Vec::as_slice).collect();
        let settings = SupportSettings {
            enabled: true,
            style: crate::Style::Tree,
            ..SupportSettings::default()
        };
        let job = Job {
            slices: &slices,
            first_layer_height: 0.2,
            layer_height: 0.2,
            settings: &settings,
            gap_above: 1,
            gap_below: 1,
            bed: None,
            annotations: &crate::NO_ANNOTATIONS,
        };
        let config = TreeConfig::new(&job).unwrap();
        let mut volumes = Volumes::new(&slices, &config).unwrap();
        volumes.precalculate(&config, 29).unwrap();
        let r = config.radius(0, 0.0);
        // Beside the block a tip's centre is kept its radius and the gap off.
        let collision = volumes.collision(r, 10, true);
        assert!(contains(collision, Point2::from_mm(10.5, 5.0)));
        assert!(!contains(collision, Point2::from_mm(10.7, 5.0)));
        // Layer 29, the layer of air under the roof, is blocked by the
        // roof above as well.
        let under = volumes.collision(r, 29, true);
        assert!(contains(under, Point2::from_mm(20.0, 5.0)));
        assert!(
            !volumes
                .avoidance(r, 20, Avoidance::Fast, false, true)
                .is_empty()
        );
        assert_eq!(volumes.misses.load(Ordering::Relaxed), 0);
        // The block stands on the bed; nothing to stand on beside it (but
        // for slivers at its corners, where the reference's two offsets
        // cut corners differently, too thin for a branch).
        assert!(volumes.placeable(r, 20).is_empty());
    }
}
