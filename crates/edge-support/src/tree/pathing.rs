//! Influence areas carried down the layers and merged
//! (TreeSupport3D.cpp `create_layer_pathing`): every branch is followed
//! not as a point but as the whole area its centre could be in, grown by
//! how far it may lean on each layer, kept out of the part and out of
//! wherever it could not get down from; two branches whose areas meet
//! become one, whose area is where both could be.

use edge_geom::{Coord, ExPolygon, Point2};
use rayon::prelude::*;

use super::config::TreeConfig;
use super::element::{AreaIncrease, Element, State};
use super::geometry::{
    MITER, Polys, area, diff, extent, intersection, move_inside_if_outside, offset,
    safe_offset_inc, safe_union, simplify, tiny_area, union2,
};
use super::volumes::{Avoidance, Volumes};
use crate::SupportError;

/// An influence area on the layer below, being worked out
/// (`SupportElementMerging`).
#[derive(Clone, Debug)]
struct Merging {
    state: State,
    parents: Vec<u32>,
    influence: Polys,
    to_bp: Polys,
    to_model: Polys,
    bbox: [Coord; 4],
}

impl Merging {
    fn set_bbox(&mut self) {
        let mut all: Vec<[Coord; 4]> = Vec::new();
        for polys in [&self.influence, &self.to_bp, &self.to_model] {
            all.extend(extent(polys));
        }
        let eps = super::config::SCALED_EPSILON;
        self.bbox = all.into_iter().fold(
            [Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN],
            |[a, b, c, d], [e, f, g, h]| [a.min(e), b.min(f), c.max(g), d.max(h)],
        );
        self.bbox = [
            self.bbox[0] - eps,
            self.bbox[1] - eps,
            self.bbox[2] + eps,
            self.bbox[3] + eps,
        ];
    }

    fn centre(&self) -> (Coord, Coord) {
        (
            (self.bbox[0] + self.bbox[2]) / 2,
            (self.bbox[1] + self.bbox[3]) / 2,
        )
    }
}

struct Grown {
    state: State,
    to_bp: Polys,
    to_model: Polys,
    increased: Polys,
}

/// `increase_single_area`: the parent's area grown as `settings` say, and
/// what of it can still get to the bed or onto the part; `None` when
/// nothing can.
#[allow(clippy::too_many_arguments)]
fn increase_single_area(
    volumes: &Volumes,
    config: &TreeConfig,
    settings: &AreaIncrease,
    layer: usize,
    parent: &Element,
    relevant_offset: &Polys,
    overspeed: Coord,
    mergelayer: bool,
) -> Result<Option<Grown>, SupportError> {
    let mut current = parent.state.propagate_down();
    if settings.increase_radius {
        current.effective_radius_height += 1;
    }
    let mut radius = current.collision_radius(config);
    let tiny = tiny_area();
    let below = layer - 1;
    let min = settings.use_min_distance;
    let increased = if settings.moves {
        let mut increased = relevant_offset.clone();
        if overspeed > 0 {
            let safe = config.safe_movement_distance(current.use_min_xy_dist);
            increased = safe_offset_inc(
                &increased,
                overspeed,
                volumes.wall_restriction(
                    parent.state.collision_radius(config),
                    layer,
                    parent.state.use_min_xy_dist,
                ),
                safe,
                safe + radius,
                1,
            )?;
        }
        if settings.no_error {
            // The reference simplifies here and drops the result
            // (`polygons_simplify` returns a copy); the simplification is
            // plainly meant, as round offsets add many vertices.
            increased = simplify(&increased, edge_geom::scaled(0.025))?;
        }
        increased
    } else {
        parent.area.clone()
    };
    let mut to_bp = Vec::new();
    let mut to_model = Vec::new();
    if mergelayer || current.to_buildplate {
        to_bp = safe_union(
            &diff(
                &increased,
                volumes.avoidance(radius, below, settings.kind, false, min),
            )?,
            &[],
        )?;
        if !current.to_buildplate && area(&to_bp) > tiny {
            // A branch marked as not reaching the bed that can after all.
            current.to_buildplate = true;
        }
    }
    if config.rests_on_model {
        if mergelayer || current.to_model_gracious {
            to_model = safe_union(
                &diff(
                    &increased,
                    volumes.avoidance(radius, below, settings.kind, true, min),
                )?,
                &[],
            )?;
        }
        if !current.to_model_gracious {
            if mergelayer && area(&to_model) >= tiny {
                current.to_model_gracious = true;
            } else {
                // It cannot get to a place to stand: it goes down beside
                // the part anyway, clear of it.
                to_model = safe_union(
                    &diff(&increased, volumes.collision(radius, below, min))?,
                    &[],
                )?;
            }
        }
    }
    let mut check = if current.to_buildplate {
        &to_bp
    } else {
        &to_model
    };
    if settings.increase_radius && area(check) > tiny {
        let valid_with_radius =
            |next_radius: Coord, radius: Coord, current: &State| -> Result<bool, SupportError> {
                if volumes.ceil_radius_xy(next_radius, min) <= volumes.ceil_radius_xy(radius, min) {
                    return Ok(true);
                }
                let check = if current.to_buildplate {
                    diff(
                        &increased,
                        volumes.avoidance(next_radius, below, settings.kind, false, min),
                    )?
                } else if config.rests_on_model {
                    diff(
                        &increased,
                        if current.to_model_gracious {
                            volumes.avoidance(next_radius, below, settings.kind, true, min)
                        } else {
                            volumes.collision(next_radius, below, min)
                        },
                    )?
                } else {
                    Vec::new()
                };
                Ok(area(&check) > tiny)
            };
        let ceil_before = volumes.ceil_radius_xy(radius, min);
        let collision_radius = current.collision_radius(config);
        if collision_radius < config.increase_radius_until_radius
            && collision_radius < current.radius(config)
        {
            let target = current
                .radius(config)
                .min(config.increase_radius_until_radius);
            let mut ceil = volumes.radius_next_ceil(radius, min);
            while ceil < target {
                let next = volumes.radius_next_ceil(ceil + 1, min);
                if !valid_with_radius(next, radius, &current)? {
                    break;
                }
                ceil = next;
            }
            let mut eff = current.effective_radius_height;
            while eff + 1 < current.distance_to_top
                && config.radius(eff + 1, current.elephant_foot_increases) <= ceil
                && config.radius(eff + 1, current.elephant_foot_increases) <= current.radius(config)
            {
                eff += 1;
            }
            current.effective_radius_height = eff;
        }
        radius = current.collision_radius(config);
        let foot_increase = (config.bp_radius_increase_per_layer
            - config.branch_radius_increase_per_layer)
            .max(0.0) as Coord;
        let planned = if foot_increase > 0 {
            (1.0f64).min(
                (config.recommended_min_radius(below as i64) - current.radius(config)) as f64
                    / foot_increase as f64,
            )
        } else {
            0.0
        };
        let increase_bp_foot = planned > 0.0 && current.to_buildplate;
        if increase_bp_foot
            && current.radius(config) >= config.branch_radius
            && current.radius(config) >= config.increase_radius_until_radius
            && valid_with_radius(
                config.radius(
                    current.effective_radius_height,
                    current.elephant_foot_increases + planned,
                ),
                radius,
                &current,
            )?
        {
            current.elephant_foot_increases += planned;
            radius = current.collision_radius(config);
        }
        if ceil_before != volumes.ceil_radius_xy(radius, min) {
            if current.to_buildplate {
                to_bp = safe_union(
                    &diff(
                        &increased,
                        volumes.avoidance(radius, below, settings.kind, false, min),
                    )?,
                    &[],
                )?;
            }
            if config.rests_on_model && (!current.to_buildplate || mergelayer) {
                to_model = safe_union(
                    &diff(
                        &increased,
                        if current.to_model_gracious {
                            volumes.avoidance(radius, below, settings.kind, true, min)
                        } else {
                            volumes.collision(radius, below, min)
                        },
                    )?,
                    &[],
                )?;
            }
        }
        check = if current.to_buildplate {
            &to_bp
        } else {
            &to_model
        };
    }
    if area(check) > tiny {
        Ok(Some(Grown {
            state: current,
            to_bp,
            to_model,
            increased,
        }))
    } else {
        Ok(None)
    }
}

/// What became of one element of the layer above.
enum Outcome {
    Grown(Box<Merging>),
    /// Nothing below: the branch is lost here.
    Lost,
}

/// `increase_areas_one_layer` for one parent.
fn increase_one(
    volumes: &Volumes,
    config: &TreeConfig,
    layer: usize,
    index: usize,
    parent: &Element,
    mergelayer: bool,
) -> Result<Outcome, SupportError> {
    let mut elem = parent.state.propagate_down();
    let wall_restriction = volumes.wall_restriction(
        parent.state.collision_radius(config),
        layer,
        parent.state.use_min_xy_dist,
    );
    let mut radius = elem.collision_radius(config);
    // Five microns over, against rounding.
    let mut extra_speed: Coord = 5;
    let mut extra_slow_speed: Coord = 0;
    let ceiled_parent_radius = volumes.ceil_radius_xy(
        parent.state.collision_radius(config),
        parent.state.use_min_xy_dist,
    );
    let projected_radius_increased = config.radius(
        parent.state.effective_radius_height + 1,
        parent.state.elephant_foot_increases,
    );
    let projected_radius_delta = projected_radius_increased - parent.state.collision_radius(config);
    let safe_movement_distance = config.safe_movement_distance(elem.use_min_xy_dist);
    // As a branch thickens its outer wall moves out, so its centre may
    // move that much further without the wall leaning more.
    if ceiled_parent_radius
        == volumes.ceil_radius_xy(projected_radius_increased, parent.state.use_min_xy_dist)
        || projected_radius_increased < config.increase_radius_until_radius
    {
        extra_speed += projected_radius_delta;
    } else {
        extra_slow_speed += projected_radius_delta
            .min((config.max_move + extra_speed) - (config.max_move_slow + extra_slow_speed));
    }
    if config.layer_start_bp_radius > layer as i64
        && config.recommended_min_radius(layer as i64 - 1)
            < config.radius(
                elem.effective_radius_height + 1,
                elem.elephant_foot_increases,
            )
    {
        if ceiled_parent_radius
            == volumes.ceil_radius_xy(
                config.radius(
                    parent.state.effective_radius_height + 1,
                    parent.state.elephant_foot_increases + 1.0,
                ),
                parent.state.use_min_xy_dist,
            )
        {
            extra_speed += config.bp_radius_increase_per_layer as Coord;
        } else {
            extra_slow_speed += (config.bp_radius_increase_per_layer as Coord)
                .min(config.max_move - (config.max_move_slow + extra_slow_speed));
        }
    }
    let fast_speed = config.max_move + extra_speed;
    let slow_speed = config.max_move_slow + extra_speed + extra_slow_speed;

    // The ways of growing the area to try, best first.
    let mut order: Vec<AreaIncrease> = Vec::new();
    let insert = |settings: AreaIncrease, back: bool, order: &mut Vec<AreaIncrease>| {
        if !order.contains(&settings) {
            if back {
                order.push(settings);
            } else {
                order.insert(0, settings);
            }
        }
    };
    let s = |kind, speed, increase_radius, no_error, use_min_distance, moves| AreaIncrease {
        kind,
        speed,
        increase_radius,
        no_error,
        use_min_distance,
        moves,
    };
    let last = elem.last_area_increase;
    let parent_moved_slow = last.speed < config.max_move;
    let avoidance_speed_mismatch = parent_moved_slow && last.kind != Avoidance::Slow;
    if last.moves
        && last.no_error
        && elem.can_use_safe_radius
        && !mergelayer
        && !avoidance_speed_mismatch
        && (elem.distance_to_top >= config.tip_layers || parent_moved_slow)
    {
        // What served the parent will most likely serve.
        let speed = if parent_moved_slow {
            slow_speed
        } else {
            fast_speed
        };
        insert(
            s(last.kind, speed, true, last.no_error, false, last.moves),
            true,
            &mut order,
        );
        insert(
            s(last.kind, speed, false, last.no_error, false, last.moves),
            true,
            &mut order,
        );
    }
    if !elem.can_use_safe_radius {
        // Not yet past every hole: try going slow without moving, and
        // else fast.
        insert(
            s(Avoidance::Slow, slow_speed, true, true, false, false),
            true,
            &mut order,
        );
        if elem.distance_to_top < config.tip_layers.div_ceil(2) {
            insert(
                s(Avoidance::Fast, slow_speed, true, true, false, false),
                true,
                &mut order,
            );
        }
        insert(
            s(Avoidance::FastSafe, fast_speed, true, true, false, false),
            true,
            &mut order,
        );
        insert(
            s(Avoidance::FastSafe, fast_speed, false, true, false, true),
            true,
            &mut order,
        );
        insert(
            s(Avoidance::Fast, fast_speed, false, true, false, true),
            true,
            &mut order,
        );
    } else {
        insert(
            s(Avoidance::Slow, slow_speed, true, true, false, true),
            true,
            &mut order,
        );
        insert(
            s(Avoidance::Slow, slow_speed, false, true, false, true),
            true,
            &mut order,
        );
        if elem.distance_to_top < config.tip_layers {
            insert(
                s(Avoidance::FastSafe, slow_speed, true, true, false, true),
                true,
                &mut order,
            );
        }
        insert(
            s(Avoidance::FastSafe, fast_speed, true, true, false, true),
            true,
            &mut order,
        );
        insert(
            s(Avoidance::FastSafe, fast_speed, false, true, false, true),
            true,
            &mut order,
        );
    }
    if elem.use_min_xy_dist {
        // Each again with the smaller gap beside overhangs.
        order = order
            .into_iter()
            .flat_map(|settings| {
                [
                    settings,
                    AreaIncrease {
                        use_min_distance: true,
                        ..settings
                    },
                ]
            })
            .collect();
    }
    let error_case = elem.to_buildplate
        || (elem.to_model_gracious
            && intersection(&parent.area, volumes.placeable(radius, layer))?.is_empty());
    if error_case {
        insert(
            s(
                Avoidance::Fast,
                fast_speed,
                false,
                false,
                elem.use_min_xy_dist,
                true,
            ),
            true,
            &mut order,
        );
    }
    if elem.distance_to_top < elem.dont_move_until && elem.can_use_safe_radius {
        insert(
            s(Avoidance::Slow, 0, true, true, false, false),
            false,
            &mut order,
        );
    }

    let offset_independent_faster = radius / safe_movement_distance
        - Coord::from(config.max_move + extra_speed < radius + safe_movement_distance)
        > super::config::round_up_divide(
            extra_speed + extra_slow_speed + config.max_move_slow,
            safe_movement_distance,
        );
    let mut offset_slow: Option<Polys> = None;
    let mut offset_fast: Option<Polys> = None;
    let mut found: Option<(Grown, AreaIncrease)> = None;
    for settings in &order {
        if settings.moves {
            if offset_slow.is_none() && (settings.speed == slow_speed || !offset_independent_faster)
            {
                // Two steps make the area rounder, which keeps clear of
                // rounding trouble with pointed areas.
                offset_slow = Some(safe_offset_inc(
                    &parent.area,
                    extra_speed + extra_slow_speed + config.max_move_slow,
                    wall_restriction,
                    safe_movement_distance,
                    if offset_independent_faster {
                        safe_movement_distance + radius
                    } else {
                        0
                    },
                    2,
                )?);
            }
            if offset_fast.is_none() && settings.speed != slow_speed {
                offset_fast = Some(if offset_independent_faster {
                    safe_offset_inc(
                        &parent.area,
                        extra_speed + config.max_move,
                        wall_restriction,
                        safe_movement_distance,
                        safe_movement_distance + radius,
                        1,
                    )?
                } else {
                    let delta = config.max_move - (config.max_move_slow + extra_slow_speed);
                    safe_offset_inc(
                        offset_slow.as_ref().map_or(&[][..], Vec::as_slice),
                        delta,
                        wall_restriction,
                        safe_movement_distance,
                        safe_movement_distance + radius,
                        1,
                    )?
                });
            }
        }
        let result = if !settings.no_error {
            // Keeping the branch at all costs: from its area, grown a hair
            // so a line has room, moving half as fast again as allowed.
            let lines: Vec<Vec<Point2>> = parent
                .area
                .iter()
                .flat_map(ExPolygon::rings)
                .map(super::geometry::closed_line)
                .collect();
            let thin =
                edge_clip::thicken(lines.iter().map(Vec::as_slice), edge_geom::scaled(0.005))?;
            let base = union2(&parent.area, &thin)?;
            let mut grown = increase_single_area(
                volumes,
                config,
                settings,
                layer,
                parent,
                &base,
                ((config.max_move + extra_speed) as f64 * 1.5) as Coord,
                mergelayer,
            )?;
            if let Some(grown) = &mut grown {
                grown.state.lost = true;
            }
            grown
        } else {
            let source = if settings.speed == slow_speed {
                &offset_slow
            } else {
                &offset_fast
            };
            let empty = Vec::new();
            increase_single_area(
                volumes,
                config,
                settings,
                layer,
                parent,
                source.as_ref().unwrap_or(&empty),
                0,
                mergelayer,
            )?
        };
        if let Some(grown) = result {
            found = Some((grown, *settings));
            break;
        }
    }
    let Some((grown, settings)) = found else {
        return Ok(Outcome::Lost);
    };
    elem = grown.state;
    radius = elem.collision_radius(config);
    elem.last_area_increase = settings;
    // Do not merge a branch that may not move, or that must get away from
    // the part first.
    let bypass_merge =
        !settings.moves || (settings.use_min_distance && elem.distance_to_top < config.tip_layers);
    if settings.moves {
        elem.dont_move_until = 0;
    } else {
        elem.result = parent.state.result;
    }
    elem.can_use_safe_radius = settings.kind != Avoidance::Fast;
    if !settings.use_min_distance {
        elem.use_min_xy_dist = false;
    }
    let influence = safe_union(
        &diff(
            &grown.increased,
            volumes.collision(radius, layer - 1, elem.use_min_xy_dist),
        )?,
        &safe_union(&grown.to_bp, &grown.to_model)?,
    )?;
    let mut merging = Merging {
        state: elem,
        parents: vec![index as u32],
        influence,
        to_bp: Vec::new(),
        to_model: Vec::new(),
        bbox: [0; 4],
    };
    if !bypass_merge {
        if merging.state.to_buildplate {
            merging.to_bp = grown.to_bp;
        }
        if config.rests_on_model {
            merging.to_model = grown.to_model;
        }
    }
    merging.set_bbox();
    Ok(Outcome::Grown(Box::new(merging)))
}

/// `merge_support_element_states`.
fn merge_states(
    first: &State,
    second: &State,
    next_position: Point2,
    layer: usize,
    config: &TreeConfig,
) -> State {
    let (target_height, target_position) = if first.target_height > second.target_height {
        (first.target_height, first.target_position)
    } else {
        (second.target_height, second.target_position)
    };
    let mut out = State {
        next_position,
        layer,
        use_min_xy_dist: first.use_min_xy_dist || second.use_min_xy_dist,
        supports_roof: first.supports_roof || second.supports_roof,
        dont_move_until: first.dont_move_until.max(second.dont_move_until),
        can_use_safe_radius: first.can_use_safe_radius || second.can_use_safe_radius,
        skip_ovalisation: false,
        target_height,
        target_position,
        effective_radius_height: first
            .effective_radius_height
            .max(second.effective_radius_height),
        distance_to_top: first.distance_to_top.max(second.distance_to_top),
        to_buildplate: first.to_buildplate && second.to_buildplate,
        to_model_gracious: first.to_model_gracious && second.to_model_gracious,
        elephant_foot_increases: 0.0,
        last_area_increase: AreaIncrease {
            kind: first
                .last_area_increase
                .kind
                .min(second.last_area_increase.kind),
            speed: first
                .last_area_increase
                .speed
                .min(second.last_area_increase.speed),
            increase_radius: first.last_area_increase.increase_radius
                || second.last_area_increase.increase_radius,
            no_error: first.last_area_increase.no_error || second.last_area_increase.no_error,
            use_min_distance: first.last_area_increase.use_min_distance
                && second.last_area_increase.use_min_distance,
            moves: first.last_area_increase.moves || second.last_area_increase.moves,
        },
        ..State::default()
    };
    if config.bp_radius_increase_per_layer > 0.0 {
        let biggest = first
            .collision_radius(config)
            .max(second.collision_radius(config));
        let foot = (biggest - out.collision_radius(config)).abs();
        let per = config.bp_radius_increase_per_layer - config.branch_radius_increase_per_layer;
        if per > 0.0 {
            out.elephant_foot_increases = foot as f64 / per;
        }
    }
    out
}

/// `merge_influence_areas_two_elements`: merges `src` into `dst` if a
/// branch can stand where both may be.
fn merge_two(
    volumes: &Volumes,
    config: &TreeConfig,
    layer: usize,
    dst: &mut Merging,
    src: &mut Merging,
) -> Result<bool, SupportError> {
    // A branch sure of a place to stand is not joined to one that is not,
    // nor one kept off the part by the full gap to one kept by less.
    if dst.state.to_model_gracious != src.state.to_model_gracious
        || dst.state.use_min_xy_dist != src.state.use_min_xy_dist
    {
        return Ok(false);
    }
    let dst_bigger = dst.state.collision_radius(config) > src.state.collision_radius(config);
    let (smaller, bigger) = if dst_bigger {
        (&*src, &*dst)
    } else {
        (&*dst, &*src)
    };
    let real_radius_delta = (bigger.state.radius(config) - smaller.state.radius(config)).abs();
    {
        let b = smaller.bbox;
        let small_box = [
            b[0] - real_radius_delta,
            b[1] - real_radius_delta,
            b[2] + real_radius_delta,
            b[3] + real_radius_delta,
        ];
        let o = bigger.bbox;
        if small_box[0] > o[2] || small_box[2] < o[0] || small_box[1] > o[3] || small_box[3] < o[1]
        {
            return Ok(false);
        }
    }
    let mut increased_to_model_radius: Coord = 0;
    let merging_to_bp = dst.state.to_buildplate && src.state.to_buildplate;
    if !merging_to_bp {
        if dst.state.to_buildplate != src.state.to_buildplate {
            // A thick branch to the bed is not merged into a thinner one
            // standing on the part.
            let rdst = dst.state.radius(config);
            let rsrc = src.state.radius(config);
            if dst.state.to_buildplate {
                if rsrc < rdst {
                    increased_to_model_radius = src.state.increased_to_model_radius + rdst - rsrc;
                }
            } else if rsrc > rdst {
                increased_to_model_radius = dst.state.increased_to_model_radius + rsrc - rdst;
            }
            if increased_to_model_radius > config.max_to_model_radius_increase {
                return Ok(false);
            }
        }
        if !dst.state.supports_roof
            && !src.state.supports_roof
            && src.state.distance_to_top.max(dst.state.distance_to_top) < config.min_dtt_to_model
        {
            return Ok(false);
        }
    }
    if !bigger.state.can_use_safe_radius && smaller.state.can_use_safe_radius {
        return Ok(false);
    }
    let use_min_radius = bigger.state.use_min_xy_dist && smaller.state.use_min_xy_dist;
    let smaller_collision_radius = smaller.state.collision_radius(config);
    let collision = volumes.collision(smaller_collision_radius, layer - 1, use_min_radius);
    let intersect_small_with_bigger = |small: &Polys, big: &Polys| -> Result<Polys, SupportError> {
        intersection(
            &safe_offset_inc(
                small,
                real_radius_delta,
                collision,
                2 * (config.xy_distance + smaller_collision_radius - 3),
                0,
                0,
            )?,
            big,
        )
    };
    let (small_areas, big_areas) = if merging_to_bp {
        (&smaller.to_bp, &bigger.to_bp)
    } else {
        (&smaller.to_model, &bigger.to_model)
    };
    let intersect = intersect_small_with_bigger(small_areas, big_areas)?;
    let tiny = tiny_area();
    if area(&intersect) <= tiny {
        return Ok(false);
    }
    if area(&offset(&intersect, -edge_geom::scaled(0.025), MITER)?) <= tiny {
        return Ok(false);
    }
    let new_position = move_inside_if_outside(&intersect, dst.state.next_position);
    let mut state = merge_states(&dst.state, &src.state, new_position, layer - 1, config);
    state.increased_to_model_radius = if increased_to_model_radius == 0 {
        dst.state
            .increased_to_model_radius
            .max(src.state.increased_to_model_radius)
    } else {
        increased_to_model_radius
    };
    let influence = safe_union(
        &intersect_small_with_bigger(&smaller.influence, &bigger.influence)?,
        &intersect,
    )?;
    let to_model = if merging_to_bp && config.rests_on_model {
        if state.to_model_gracious {
            safe_union(
                &intersect_small_with_bigger(&smaller.to_model, &bigger.to_model)?,
                &intersect,
            )?
        } else {
            influence.clone()
        }
    } else {
        Vec::new()
    };
    let parents = std::mem::take(&mut src.parents);
    dst.parents.extend(parents);
    dst.state = state;
    dst.influence = influence;
    if merging_to_bp {
        dst.to_bp = intersect;
        dst.to_model = if config.rests_on_model {
            to_model
        } else {
            Vec::new()
        };
    } else {
        dst.to_bp = Vec::new();
        dst.to_model = intersect;
    }
    dst.set_bbox();
    src.influence.clear();
    src.to_bp.clear();
    src.to_model.clear();
    Ok(true)
}

/// Every pair in a small group checked against each other.
fn merge_leaves(
    volumes: &Volumes,
    config: &TreeConfig,
    layer: usize,
    group: &mut Vec<Merging>,
) -> Result<(), SupportError> {
    let mut i = 0;
    while i + 1 < group.len() {
        let mut merged = false;
        let mut j = i + 1;
        while j < group.len() {
            let (head, tail) = group.split_at_mut(j);
            if merge_two(volumes, config, layer, &mut head[i], &mut tail[0])? {
                group.swap_remove(j);
                merged = true;
                break;
            }
            j += 1;
        }
        if !merged {
            i += 1;
        }
    }
    Ok(())
}

/// One group merged into another (`merge_influence_areas_two_sets`).
fn merge_sets(
    volumes: &Volumes,
    config: &TreeConfig,
    layer: usize,
    dst: &mut Vec<Merging>,
    src: Vec<Merging>,
) -> Result<(), SupportError> {
    let mut left = Vec::new();
    for mut item in src {
        let mut merged_at = None;
        for (at, target) in dst.iter_mut().enumerate() {
            if merge_two(volumes, config, layer, target, &mut item)? {
                merged_at = Some(at);
                break;
            }
        }
        let Some(at) = merged_at else {
            left.push(item);
            continue;
        };
        let mut j = at + 1;
        while j < dst.len() {
            let (head, tail) = dst.split_at_mut(j);
            if merge_two(volumes, config, layer, &mut head[at], &mut tail[0])? {
                dst.swap_remove(j);
            } else {
                j += 1;
            }
        }
    }
    dst.extend(left);
    Ok(())
}

/// Puts areas near each other next to each other, as the reference's
/// bounding box tree orders them: split at the median along the wider
/// extent of their centres, each half again, and so on.
fn order_by_place(items: &mut [Merging]) {
    if items.len() <= 1 {
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (Coord::MAX, Coord::MAX, Coord::MIN, Coord::MIN);
    for item in items.iter() {
        let (x, y) = item.centre();
        (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
    }
    let by_x = x1 - x0 >= y1 - y0;
    let key = |item: &Merging| {
        let (x, y) = item.centre();
        if by_x { (x, y) } else { (y, x) }
    };
    let middle = items.len() / 2;
    items.select_nth_unstable_by(middle, |a, b| {
        key(a).cmp(&key(b)).then_with(|| a.parents.cmp(&b.parents))
    });
    let (left, right) = items.split_at_mut(middle);
    order_by_place(left);
    order_by_place(right);
}

/// `merge_influence_areas`: neighbours in groups of four, then each group
/// with the next, and so on.
fn merge_all(
    volumes: &Volumes,
    config: &TreeConfig,
    layer: usize,
    mut items: Vec<Merging>,
) -> Result<Vec<Merging>, SupportError> {
    if items.len() <= 1 {
        return Ok(items);
    }
    order_by_place(&mut items);
    let mut buckets: Vec<Vec<Merging>> = Vec::new();
    let initial = (items.len() + 2) / 4;
    let mut rest = items.into_iter();
    for _ in 0..initial.max(1) {
        let bucket: Vec<Merging> = rest.by_ref().take(4).collect();
        if !bucket.is_empty() {
            buckets.push(bucket);
        }
    }
    let leftover: Vec<Merging> = rest.collect();
    let last_single = !leftover.is_empty();
    if last_single {
        buckets.push(leftover);
    }
    let first_round = if last_single {
        buckets.len() - 1
    } else {
        buckets.len()
    };
    buckets[..first_round]
        .par_iter_mut()
        .try_for_each(|bucket| merge_leaves(volumes, config, layer, bucket))?;
    while buckets.len() > 1 {
        let mut pairs: Vec<(Vec<Merging>, Option<Vec<Merging>>)> = Vec::new();
        let mut it = buckets.into_iter();
        while let Some(a) = it.next() {
            pairs.push((a, it.next()));
        }
        buckets = pairs
            .into_par_iter()
            .map(|(mut a, b)| {
                if let Some(b) = b {
                    merge_sets(volumes, config, layer, &mut a, b)?;
                }
                Ok(a)
            })
            .collect::<Result<_, SupportError>>()?;
    }
    Ok(buckets.pop().unwrap_or_default())
}

/// Carries the tips' influence areas down to the bed, merging them on the
/// way (`create_layer_pathing`). `layers` holds the tips on entry and
/// every element on every layer on return.
pub(crate) fn create_layer_pathing(
    volumes: &Volumes,
    config: &TreeConfig,
    layers: &mut [Vec<Element>],
) -> Result<(), SupportError> {
    let count = layers.len();
    let mut last_merge_layer = count;
    let mut new_element = false;
    // At least one merge every 3 mm of height, 50 layers, 1 mm of slow or
    // 5 mm of fast movement, whichever comes first.
    let max_merge_every = (5_000_000 / config.max_move.max(100_000))
        .min(1_000_000 / config.max_move_slow.max(20_000))
        .min(3_000_000 / config.layer_height.max(1))
        .max(1) as usize;
    let mut merge_every = 1usize;
    for layer in (1..count).rev() {
        if layers[layer].is_empty() {
            continue;
        }
        let had_new = new_element;
        let merge_this_layer = had_new || last_merge_layer - layer >= merge_every;
        if had_new {
            merge_every = 1;
        }
        let outcomes = layers[layer]
            .par_iter()
            .enumerate()
            .map(|(index, parent)| {
                increase_one(volumes, config, layer, index, parent, merge_this_layer)
            })
            .collect::<Result<Vec<Outcome>, SupportError>>()?;
        let mut influence: Vec<Merging> = Vec::new();
        let mut done: Vec<Element> = Vec::new();
        for (index, outcome) in outcomes.into_iter().enumerate() {
            match outcome {
                Outcome::Lost => {
                    // The branch's lowest point is left unplaced, so that
                    // one too short to stand on the part can be dropped.
                    let parent = &mut layers[layer][index].state;
                    parent.result = None;
                    parent.to_model_gracious = false;
                    parent.verylost = true;
                }
                Outcome::Grown(merging) => {
                    if merging.influence.is_empty() {
                        continue;
                    }
                    if merging.to_bp.is_empty() && merging.to_model.is_empty() {
                        done.push(Element {
                            state: merging.state,
                            parents: merging.parents,
                            area: merging.influence,
                        });
                    } else {
                        influence.push(*merging);
                    }
                }
            }
        }
        new_element = !layers[layer - 1].is_empty() || !done.is_empty();
        if merge_this_layer {
            let before = influence.len();
            if before > 1 {
                influence = merge_all(volumes, config, layer, influence)?;
            }
            last_merge_layer = layer;
            if before <= influence.len() && !had_new {
                merge_every = (merge_every + 1).min(max_merge_every);
            }
        }
        let below = &mut layers[layer - 1];
        below.extend(done);
        for merging in influence {
            if merging.influence.is_empty() {
                continue;
            }
            // The reference warns of an area this small and keeps it.
            let united = safe_union(&merging.influence, &[])?;
            below.push(Element {
                state: merging.state,
                parents: merging.parents,
                area: united,
            });
        }
    }
    Ok(())
}
