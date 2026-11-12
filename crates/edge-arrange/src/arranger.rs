//! The reference's arranger (Arrange.cpp `AutoArranger`, `_arrange`,
//! `arrange`) over libnest2d's first-fit selection
//! (`selections/firstfit.hpp` `_FirstFitSelection::packItems`).

use std::collections::BTreeSet;

use edge_geom::{Coord, Point2};

use crate::geom::{Bbox, distance};
use crate::item::{Item, UNFIT, UNSET};
use crate::params::{MAX_NUM_PLATES, Params, Piece};
use crate::placer::{Align, Objective, PackResult, Pile, Placer, PlacerConfig, Scorer, overfit};
use crate::rotation::{fit_into_box_rotation, min_area_rotation};

/// A score no place may reach to be taken (nester.hpp).
pub const LARGE_COST_TO_REJECT: f64 = 1e7;
/// Items over this share of the bed's area are big (Arrange.cpp l.82).
const BIG_ITEM_TRESHOLD: f64 = 0.02;

/// What the objective knows of the whole job.
struct AutoArranger<'a> {
    params: &'a Params,
    bin_area: f64,
    norm: f64,
    config: PlacerConfig,
    /// How many items are to be placed, for how much is left
    /// (`m_item_count`).
    item_count: usize,
}

/// libnest2d's `FilamentTempType::HighTemp` and `LowTemp`.
const HIGH_TEMP: i32 = 0;
const LOW_TEMP: i32 = 1;

/// Print.cpp `is_filaments_compatible` for two items.
fn compatible(a: i32, b: i32) -> bool {
    let high = a == HIGH_TEMP || b == HIGH_TEMP;
    let low = a == LOW_TEMP || b == LOW_TEMP;
    !(high && low)
}

impl AutoArranger<'_> {
    fn norm(&self, value: f64) -> f64 {
        value / self.norm
    }

    fn is_big(&self, area: f64) -> bool {
        area / self.bin_area > BIG_ITEM_TRESHOLD
    }

    /// Printing by object: how far up the bed the item's corner is, much,
    /// and across it, little, and how many rows it takes
    /// (`dist_for_BOTTOM_LEFT`).
    fn dist_for_bottom_left(&self, ibb: &Bbox, origin: Point2) -> f64 {
        let dy = (ibb.min.y - origin.y) as f64;
        let dx = (ibb.min.x - origin.x) as f64;
        let mut bindist = (ibb.max.y - ibb.min.y) as f64;
        if dx >= 0.0 && dy >= 0.0 {
            bindist += dy + dx;
        } else {
            if dx < 0.0 {
                bindist += 10.0 * -dx;
            }
            if dy < 0.0 {
                bindist += 10.0 * -dy;
            }
        }
        self.norm(bindist)
    }

    fn dist_to_bin(&self, ibb: &Bbox, origin: Point2) -> f64 {
        match self.config.starting_point {
            Align::BottomLeft => self.norm(distance(ibb.min, origin)),
            Align::TopRight => self.norm(distance(ibb.max, origin)),
            _ => self.norm(distance(ibb.center(), origin)),
        }
    }

    /// The objective (`AutoArranger::objfunc`, Arrange.cpp l.452): the
    /// score of the item where it stands, and the box of the pile with it.
    fn objfunc(&self, item: &Item, origin: Point2, pile: &Pile<'_>) -> (f64, Bbox) {
        let params = self.params;
        let ibb = item.bounding_box();
        let fullbb = pile.pile_box.merge(ibb);
        let bigbb = if pile.big.is_empty() {
            fullbb
        } else {
            pile.big
                .iter()
                .fold(Bbox::default(), |bb, (b, _)| bb.merge(*b))
        };
        let mut score;
        let big_item = self.is_big(item.area()) || pile.big.is_empty();
        let remaining_empty = pile.remaining == 0;
        enum Case {
            Big,
            LastBig,
            Small,
        }
        let case = if !params.is_seq_print && big_item && !remaining_empty {
            Case::Big
        } else if big_item && remaining_empty {
            Case::LastBig
        } else {
            Case::Small
        };
        match case {
            Case::Big => {
                let (minc, maxc) = (ibb.min, ibb.max);
                let top_left = Point2 {
                    x: minc.x,
                    y: maxc.y,
                };
                let bottom_right = Point2 {
                    x: maxc.x,
                    y: minc.y,
                };
                let cc = fullbb.center();
                let dists = [
                    distance(minc, cc),
                    distance(maxc, cc),
                    distance(ibb.center(), cc),
                    distance(top_left, cc),
                    distance(bottom_right, cc),
                ];
                let mut dist = self.norm(dists.iter().copied().fold(f64::INFINITY, f64::min));
                if self.config.starting_point == Align::BottomLeft {
                    let bindist = self.dist_for_bottom_left(&ibb, origin);
                    score = 0.2 * dist + 0.8 * bindist;
                } else {
                    let bindist = self.dist_to_bin(&ibb, origin);
                    dist = 0.8 * dist + 0.2 * bindist;
                    // How well it lines up with a neighbour of the same
                    // area: the best of them.
                    let mut alignment_score = 1.0;
                    let index = if self.is_big(item.area()) {
                        &pile.big
                    } else {
                        &pile.all
                    };
                    let mut neighbours = 0;
                    for (b, i) in index {
                        if !b.intersects(&ibb) {
                            continue;
                        }
                        neighbours += 1;
                        let p = &pile.arena[*i];
                        let parea = p.area();
                        if (1.0 - parea / item.area()).abs() < 1e-6 {
                            let bb = p.bounding_box().merge(ibb);
                            let ascore = 1.0 - (item.area() + parea) / bb.area();
                            if ascore < alignment_score {
                                alignment_score = ascore;
                            }
                        }
                    }
                    let density = (self.norm(fullbb.width() as f64)
                        * self.norm(fullbb.height() as f64))
                    .sqrt();
                    let r = pile.remaining as f64 / self.item_count.max(1) as f64;
                    let alignment_weight = (0.6 * item.area() / ibb.area()).max(0.3);
                    score = if neighbours == 0 {
                        0.50 * dist + 0.50 * density
                    } else {
                        (1.0 - 0.2 - alignment_weight) * dist
                            + (1.0 - r) * 0.20 * density
                            + alignment_weight * alignment_score
                    };
                }
            }
            Case::LastBig => {
                score = if self.config.starting_point == Align::BottomLeft {
                    self.dist_for_bottom_left(&ibb, origin)
                } else if pile.pile_box.defined {
                    0.5 * self.norm(distance(ibb.center(), pile.pile_box.center()))
                } else {
                    0.5 * self.norm(distance(ibb.center(), origin))
                };
            }
            Case::Small => {
                score = if self.config.starting_point == Align::BottomLeft {
                    self.dist_for_bottom_left(&ibb, origin)
                } else {
                    0.8 * self.norm(distance(ibb.center(), bigbb.center()))
                        + 0.2 * self.norm(distance(ibb.center(), origin))
                };
            }
        }

        let placed = pile
            .items
            .iter()
            .map(|&i| &pile.arena[i])
            .filter(|p| !p.is_virtual);
        if params.is_seq_print {
            let (rod, lid) = (
                params.clearance_height_to_rod,
                params.clearance_height_to_lid,
            );
            let mut row_conflict = false;
            let mut lid_conflict = false;
            let (iy1, iy2, ix1) = (ibb.min.y, ibb.max.y, ibb.min.x);
            for p in placed.clone() {
                let pbb = p.bounding_box();
                let (px1, py1, py2) = (pbb.min.x, pbb.min.y, pbb.max.y);
                let (inter_min, inter_max) = (iy1.max(py1), iy2.min(py2));
                // Lower on the bed is printed first, under the rod.
                if iy2 < py1 {
                    row_conflict |= item.height > rod;
                } else if inter_max - inter_min > 0 {
                    // Side by side: the left one first.
                    let h = if ix1 < px1 { item.height } else { p.height };
                    row_conflict |= h > rod;
                }
                // Only the last may be taller than the lid allows.
                lid_conflict |= p.height > lid;
            }
            let lambda3 = LARGE_COST_TO_REJECT * 1.1;
            let lambda4 = LARGE_COST_TO_REJECT * 1.2;
            for p in placed.clone() {
                if !compatible(item.filament_temp_type, p.filament_temp_type) {
                    score += lambda3;
                }
            }
            score += lambda4 * f64::from(u8::from(row_conflict))
                + lambda4 * f64::from(u8::from(lid_conflict));
        } else {
            // Objects of like height near each other.
            let mut count = 0;
            let mut height_score = 0.0;
            for p in placed.clone() {
                count += 1;
                height_score += (1.0 - (item.height - p.height).abs() / params.printable_height)
                    * self.norm(distance(ibb.center(), p.bounding_box().center()));
                if !compatible(item.filament_temp_type, p.filament_temp_type) {
                    score += LARGE_COST_TO_REJECT;
                    break;
                }
            }
            if count > 0 {
                score += height_score / f64::from(count);
            }
        }

        let mut filaments: BTreeSet<i32> = BTreeSet::new();
        for p in placed {
            filaments.extend(p.filaments.iter().copied());
        }
        if !params.allow_multi_materials_on_same_plate {
            let first = filaments.is_empty();
            let own: BTreeSet<i32> = item.filaments.iter().copied().collect();
            // Alike when either's filaments are among the other's.
            let same = filaments.is_subset(&own) || own.is_subset(&filaments);
            if !(first || same) {
                score += LARGE_COST_TO_REJECT * 1.3;
            }
        }
        // As few filament changes as may be: a weak pull.
        let before = filaments.len();
        filaments.extend(item.filaments.iter().copied());
        if !params.is_seq_print {
            score += (filaments.len() - before) as f64;
        }
        (score, fullbb)
    }
}

impl Scorer for AutoArranger<'_> {
    fn score(&self, item: &Item, pile: &Pile<'_>, objective: &Objective, bin: &Bbox) -> f64 {
        match objective {
            Objective::Normal { origin } => {
                // `AutoArranger<Box>::get_objfn`.
                let (mut score, fullbb) = self.objfunc(item, *origin, pile);
                let miss = overfit(&fullbb, bin).max(0.0);
                score += miss * miss;
                if score > LARGE_COST_TO_REJECT {
                    score = 1.5 * LARGE_COST_TO_REJECT;
                }
                score
            }
            Objective::Fixed { start } => {
                // `fixed_overfit`.
                let (mut score, pilebb) = self.objfunc(item, *start, pile);
                let full = pilebb.merge(*bin);
                let diff = full.area() - bin.area();
                if diff > 0.0 {
                    score += diff;
                }
                score
            }
        }
    }

    fn is_big(&self, area: f64) -> bool {
        AutoArranger::is_big(self, area)
    }
}

/// The order items are tried in (`sortfunc`, Arrange.cpp l.816): by
/// priority, then the hotter bed first, then printing by layer the lower
/// first filament and the larger, by object the shorter and the larger.
/// The reference sorts unstably, so items alike in all of these go in an
/// order it leaves to chance; here they keep the order they were given in.
fn sort_items(store: &mut [usize], arena: &[Item], seq: bool) {
    store.sort_by(|&a, &b| {
        let (i1, i2) = (&arena[a], &arena[b]);
        use std::cmp::Ordering;
        let before = |x: bool, y: bool| match (x, y) {
            (true, _) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => Ordering::Equal,
        };
        if i1.priority != i2.priority {
            return i2.priority.cmp(&i1.priority);
        }
        if i1.bed_temp != i2.bed_temp {
            return i2.bed_temp.total_cmp(&i1.bed_temp);
        }
        if seq {
            if i1.height != i2.height {
                return i1.height.total_cmp(&i2.height);
            }
        } else if i1.filaments != i2.filaments {
            let (f1, f2) = (
                i1.filaments.first().copied().unwrap_or(0),
                i2.filaments.first().copied().unwrap_or(0),
            );
            // Unequal lists with the same first filament are equivalent.
            return before(f1 < f2, f2 < f1);
        }
        i2.area().total_cmp(&i1.area())
    });
}

/// libnest2d's first-fit selection with the reference's changes
/// (firstfit.hpp `packItems`): each item, largest first, goes on the bed
/// it scores best on, a new bed being opened when none takes it.
#[allow(clippy::too_many_arguments)]
fn pack_items(
    arena: &mut [Item],
    shapes: &[usize],
    fixed: &[usize],
    excluded: &[usize],
    bin: Bbox,
    arranger: &AutoArranger<'_>,
    normal: Objective,
) {
    let config = &arranger.config;
    // Fixed items by bed; the others to place.
    let mut fixed_bins: Vec<Vec<usize>> = Vec::new();
    for &i in fixed {
        if arena[i].bin < 0 {
            arena[i].bin = 0;
        }
        let bin_index = arena[i].bin as usize;
        while fixed_bins.len() <= bin_index {
            fixed_bins.push(Vec::new());
        }
        fixed_bins[bin_index].push(i);
    }
    let mut store: Vec<usize> = shapes.to_vec();
    sort_items(&mut store, arena, arranger.params.is_seq_print);

    // An item no empty bed holds (with the bed's excluded regions) is not
    // tried (`remove_unpackable_items`).
    {
        let mut probe_bed = Placer::new(bin, normal);
        probe_bed.preload(excluded, arena, config);
        for &i in &store {
            let mut copy = arena[i].clone();
            let result = probe_bed.pack(&mut copy, arena, 0, config, arranger);
            if arena[i].area() <= 0.0 || !result.packed {
                arena[i].bin = UNFIT;
            }
        }
    }

    let mut placers: Vec<Placer> = Vec::new();
    let mut item_id = 0;
    for (k, &index) in store.iter().enumerate() {
        if arena[index].bin == UNFIT {
            continue;
        }
        let remaining = store.len() - k - 1;
        let mut was_packed = false;
        let mut best_bed: Option<usize> = None;
        let mut first_fit: Option<(usize, PackResult)> = None;
        let mut best_result: Option<PackResult> = None;
        let mut best_all_plates = f64::MAX;
        let mut j = 0;
        // Each item is tried from the turn it was given on every bed. The
        // reference starts each bed from where the last bed's search left
        // the item (firstfit.hpp l.131 passes the item itself, which
        // `_trypack` turns), so the turns it tries on one bed depend on the
        // bed before.
        let initial_rotation = arena[index].rotation();
        let initial_translation = arena[index].translation();
        // An item an empty new bed does not take is put on none rather
        // than opening beds without end (see the error noted in
        // UI_PARITY.md): `remove_unpackable_items` tries it without the
        // prime tower, which a new bed may carry.
        let mut empty_beds_tried = 0;
        while !was_packed {
            while j < placers.len() && !was_packed {
                let mut trial = arena[index].clone();
                trial.set_rotation(initial_rotation);
                trial.set_translation(initial_translation);
                let result = placers[j].pack(&mut trial, arena, remaining, config, arranger);
                let score = result.score;
                let all_plates = score + placers.iter().map(|p| p.score).sum::<f64>();
                if (0.0..LARGE_COST_TO_REJECT).contains(&score) {
                    if first_fit.is_none() {
                        first_fit = Some((j, result));
                    }
                    if all_plates < best_all_plates {
                        best_all_plates = all_plates;
                        best_bed = Some(j);
                        best_result = Some(result);
                    }
                }
                j += 1;
            }
            if best_bed.is_none() && placers.len() >= MAX_NUM_PLATES {
                // Every bed there may be was tried: take the first that
                // took it at all, else it fits nowhere.
                match first_fit {
                    Some((bed, result)) => {
                        best_bed = Some(bed);
                        best_result = Some(result);
                    }
                    None => {
                        arena[index].bin = UNFIT;
                        break;
                    }
                }
            }
            if let (Some(bed), Some(result)) = (best_bed, best_result) {
                was_packed = true;
                j = bed;
                arena[index].bin = bed as i32;
                arena[index].item_id = item_id;
                item_id += 1;
                placers[bed].accept(&result, index, arena);
            }
            if was_packed && arena[index].tried_with_excluded {
                placers[j].clear_items(arena, config, |itm| itm.fixed && !itm.is_wipe_tower);
                if fixed_bins.len() >= placers.len() {
                    let group = fixed_bins[placers.len() - 1].clone();
                    placers[j].preload(&group, arena, config);
                }
            }
            let last_empty = !was_packed
                && !placers.is_empty()
                && j == placers.len()
                && placers[j - 1].packed(arena) == 0;
            if last_empty {
                if !arena[index].tried_with_excluded {
                    // A large item an empty bed did not take: try it with
                    // only the bed's excluded regions, which are smaller
                    // than what stands for them otherwise.
                    arena[index].tried_with_excluded = true;
                    placers[j - 1]
                        .clear_items(arena, config, |itm| itm.fixed && !itm.is_wipe_tower);
                    placers[j - 1].preload(excluded, arena, config);
                    j -= 1;
                    continue;
                }
                placers[j - 1].clear_items(arena, config, |itm| itm.fixed && !itm.is_wipe_tower);
                if let Some(group) = fixed_bins.get(placers.len() - 1).cloned() {
                    placers[j - 1].preload(&group, arena, config);
                }
                empty_beds_tried += 1;
                if empty_beds_tried > 1 {
                    arena[index].bin = UNFIT;
                    break;
                }
            }
            if !was_packed {
                if placers.len() >= MAX_NUM_PLATES {
                    arena[index].bin = UNFIT;
                    break;
                }
                let mut placer = Placer::new(bin, normal);
                if let Some(group) = fixed_bins.get(placers.len()).cloned() {
                    placer.preload(&group, arena, config);
                }
                placers.push(placer);
                j = placers.len() - 1;
            }
        }
    }
    // The beds go as the reference's placers go out of scope, each lining
    // up its pile.
    for placer in placers.iter().rev() {
        placer.final_align(arena, config);
    }
    for &i in &store {
        if arena[i].bin == UNFIT {
            arena[i].bin = UNSET;
        }
    }
}

fn item_of(piece: &Piece) -> Item {
    let mut item = Item::new(piece.outline.clone());
    item.set_rotation(piece.rotation);
    item.set_translation(piece.translation);
    item.bin = piece.bed;
    item.priority = piece.priority;
    item.item_id = piece.item_id;
    item.filaments = piece.filaments.clone();
    item.height = piece.height;
    item.name = piece.name.clone();
    item.is_virtual = piece.is_virtual;
    item.is_wipe_tower = piece.is_wipe_tower;
    item.bed_temp = f64::from(piece.first_bed_temp);
    item.set_inflation(piece.inflation);
    item.filament_temp_type = piece.filament_temp_type;
    item
}

/// Arranges `items` on a box bed `bed` (`[min_x, min_y, max_x, max_y]`,
/// nanometres), around `fixed` (each on the bed its `bed` names, which
/// stays where it is), as the reference's `arrangement::arrange` does on a
/// rectangular bed (Arrange.cpp l.1134 and `_arrange`, l.979). Each item
/// comes back with its translation, rotation, bed (-1: no bed takes it)
/// and the order it was placed in (`item_id`).
pub fn arrange(items: &mut [Piece], fixed: &[Piece], bed: [Coord; 4], params: &Params) {
    let bin = Bbox::new(
        Point2 {
            x: bed[0],
            y: bed[1],
        },
        Point2 {
            x: bed[2],
            y: bed[3],
        },
    );
    let mut arena: Vec<Item> = Vec::new();
    // Outlines of fewer than three corners take no place. The reference
    // drops them from its list and then writes its results back by
    // position (Arrange.cpp l.1079 and l.1154), so each item after one of
    // them would get the next one's place; here they are left unarranged.
    let mut shapes = Vec::new();
    let mut owners = Vec::new();
    for (k, piece) in items.iter_mut().enumerate() {
        if piece.outline.points.len() < 3 {
            piece.bed = UNSET;
            continue;
        }
        shapes.push(arena.len());
        owners.push(k);
        arena.push(item_of(piece));
    }
    let mut fixed_items = Vec::new();
    for piece in fixed {
        if piece.outline.points.len() < 3 {
            continue;
        }
        let mut item = item_of(piece);
        // `itm.inflate(scaled(-2. * EPSILON))`.
        item.inflate(-200);
        // `remove_items_outside_bin`: a fixed object wholly off the bed (on
        // another plate) is dropped.
        let bb = item.bounding_box();
        if !item.is_virtual
            && (bb.max.x < bin.min.x
                || bb.min.x > bin.max.x
                || bb.max.y < bin.min.y
                || bb.min.y > bin.max.y)
        {
            continue;
        }
        item.mark_fixed(item.bin);
        fixed_items.push(arena.len());
        arena.push(item);
    }
    // The bed's excluded regions, which the selection tries a large item
    // against when it fits nowhere else (`fill_config`).
    let mut excluded = Vec::new();
    for piece in params
        .excluded_regions
        .iter()
        .chain(&params.nonprefered_regions)
    {
        if piece.outline.points.len() < 3 {
            continue;
        }
        let mut item = item_of(piece);
        item.mark_fixed(0);
        item.inflate(-200);
        excluded.push(arena.len());
        arena.push(item);
    }

    // `fill_config` and the `AutoArranger` constructor.
    let starting_point = if params.is_seq_print {
        Align::BottomLeft
    } else {
        Align::TopRight
    };
    let mut alignment = if params.do_final_align {
        Align::Center
    } else {
        Align::Unaligned
    };
    let mut best_object_pos = Point2::default();
    if (params.align_center[0] - 0.5).abs() > 0.001 || (params.align_center[1] - 0.5).abs() > 0.001
    {
        best_object_pos = Point2 {
            x: bin.min.x + (bin.width() as f64 * params.align_center[0]) as Coord,
            y: bin.min.y + (bin.height() as f64 * params.align_center[1]) as Coord,
        };
        alignment = Align::UserDefined;
    }
    let rotations = if params.allow_rotations {
        use std::f64::consts::PI;
        vec![0.0, PI / 4.0, PI / 2.0, 3.0 * PI / 4.0]
    } else {
        vec![0.0]
    };
    let config = PlacerConfig {
        rotations,
        alignment,
        starting_point,
        best_object_pos,
        accuracy: params.accuracy,
    };
    let bin_area = bin.area().abs();
    // The reference counts every item it is handed, the fixed ones twice
    // over (Arrange.cpp l.835 and l.849); how much is left to place is a
    // share of what is to be placed.
    let arranger = AutoArranger {
        params,
        bin_area,
        norm: bin_area.sqrt(),
        config,
        item_count: shapes.len(),
    };
    // `AutoArranger<Box>::get_objfn`'s origin.
    let origin = match arranger.config.starting_point {
        Align::Center => bin.center(),
        Align::TopRight => bin.max,
        _ => bin.min,
    };

    // Turned to the smallest box round them, and to fit when too big.
    if params.allow_rotations {
        for &i in &shapes {
            let turn = min_area_rotation(&arena[i].transformed());
            // The reference sets this as the item's whole turn, measured on
            // the outline already turned by its own (Arrange.cpp l.1013);
            // it is a turn more.
            let total = arena[i].rotation() + turn;
            arena[i].set_rotation(total);
            let bb = arena[i].bounding_box();
            if bb.width() >= bin.width() || bb.height() >= bin.height() {
                let more = fit_into_box_rotation(&arena[i].transformed(), &bin);
                let total = arena[i].rotation() + more;
                arena[i].set_rotation(total);
            }
        }
    }
    pack_items(
        &mut arena,
        &shapes,
        &fixed_items,
        &excluded,
        bin,
        &arranger,
        Objective::Normal { origin },
    );

    for (slot, &k) in shapes.iter().zip(&owners) {
        let item = &arena[*slot];
        let piece = &mut items[k];
        piece.translation = item.translation();
        piece.rotation = item.rotation();
        piece.bed = item.bin;
        piece.item_id = item.item_id;
    }
}
