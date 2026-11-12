//! Placing one item on one bed (libnest2d `placers/nfpplacer.hpp`,
//! `_NofitPolyPlacer`): where the no-fit polygons of what is on the bed
//! leave room, the place along their outlines the objective scores best.

use edge_geom::{ExPolygon, Point2, Polygon, convex_hull};

use crate::edges::EdgeCache;
use crate::geom::{Bbox, Side, add, projection_onto, region_side, sub};
use crate::item::Item;
use crate::subplex;

/// Where the first item goes and where the finished pile is put
/// (`NfpPConfig::Alignment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Center,
    BottomLeft,
    TopRight,
    Unaligned,
    UserDefined,
}

/// The placer's settings (`NfpPConfig`).
#[derive(Clone, Debug)]
pub struct PlacerConfig {
    pub rotations: Vec<f64>,
    pub alignment: Align,
    pub starting_point: Align,
    pub best_object_pos: Point2,
    pub accuracy: f32,
}

/// Which objective a bed scores places with: the arranger's own, or, once
/// fixed items are on the bed, the one `on_preload` puts in its place
/// (Arrange.cpp l.770).
#[derive(Clone, Copy, Debug)]
pub enum Objective {
    /// `AutoArranger<Box>::get_objfn`: scored from `origin`, a pile bigger
    /// than the bed paying for it.
    Normal { origin: Point2 },
    /// `fixed_overfit(objfunc(item, start), bin)`.
    Fixed { start: Point2 },
}

/// What scoring a place needs to know of the bed as it stands
/// (`before_packing`, Arrange.cpp l.735).
pub struct Pile<'a> {
    pub arena: &'a [Item],
    /// The items on the bed, fixed ones included.
    pub items: &'a [usize],
    /// The box round what has been placed (not the fixed items).
    pub pile_box: Bbox,
    /// The boxes of the big items and of all items, with the item.
    pub big: Vec<(Bbox, usize)>,
    pub all: Vec<(Bbox, usize)>,
    /// How many items are still to come after this one.
    pub remaining: usize,
}

/// Scores an item standing where it is against a pile, with whichever
/// objective the bed uses.
pub trait Scorer {
    fn score(&self, item: &Item, pile: &Pile<'_>, objective: &Objective, bin: &Bbox) -> f64;
    fn is_big(&self, area: f64) -> bool;
}

/// What trying an item on a bed came to (`PackResult`).
#[derive(Clone, Copy, Debug)]
pub struct PackResult {
    pub packed: bool,
    pub translation: Point2,
    pub rotation: f64,
    pub score: f64,
}

impl PackResult {
    fn failed() -> Self {
        Self {
            packed: false,
            translation: Point2::default(),
            rotation: 0.0,
            score: -1.11,
        }
    }
}

/// One bed being filled.
#[derive(Clone, Debug)]
pub struct Placer {
    pub bin: Bbox,
    /// What is on it, as places in the arena; fixed items first.
    pub items: Vec<usize>,
    merged_pile: Vec<ExPolygon>,
    pub score: f64,
    pub objective: Objective,
}

/// The excess of a box over the bed's size, summed across and up
/// (`overfit(Box, Box)`).
pub fn overfit(bb: &Bbox, bin: &Bbox) -> f64 {
    let mut diff = 0.0;
    let (w, h) = (bb.width() - bin.width(), bb.height() - bin.height());
    if w > 0 {
        diff += w as f64;
    }
    if h > 0 {
        diff += h as f64;
    }
    diff
}

/// The free places on a bed for `sliding`'s reference vertex, clear of
/// each of `stationary` (`calcnfp`).
fn free_region(sliding: &Polygon, stationary: &[Polygon], bin: &Bbox) -> Vec<ExPolygon> {
    let Some(inner) =
        edge_clip::inner_fit_rectangle([bin.min.x, bin.min.y, bin.max.x, bin.max.y], sliding)
    else {
        return Vec::new();
    };
    let taken: Vec<ExPolygon> = stationary
        .iter()
        .map(|s| ExPolygon {
            contour: edge_clip::nfp_convex(s, sliding),
            holes: Vec::new(),
        })
        .filter(|nfp| nfp.contour.points.len() >= 3)
        .collect();
    let inner = ExPolygon {
        contour: inner,
        holes: Vec::new(),
    };
    if taken.is_empty() {
        return vec![inner];
    }
    edge_clip::difference(&[inner], &taken).unwrap_or_default()
}

impl Placer {
    pub fn new(bin: Bbox, objective: Objective) -> Self {
        Self {
            bin,
            items: Vec::new(),
            merged_pile: Vec::new(),
            score: 0.0,
            objective,
        }
    }

    /// How many items have been placed (not fixed) (`getPackedSize`).
    pub fn packed(&self, arena: &[Item]) -> usize {
        self.items.iter().filter(|&&i| !arena[i].fixed).count()
    }

    /// Puts fixed items on the bed (`preload` and `on_preload`): from then
    /// on places are scored from the bed's middle, or from the prime
    /// tower's when one is among them, or from the bed's corner printing
    /// by object.
    pub fn preload(&mut self, group: &[usize], arena: &[Item], config: &PlacerConfig) {
        self.items.extend_from_slice(group);
        if group.is_empty() {
            return;
        }
        let mut start = if config.starting_point == Align::BottomLeft {
            self.bin.min
        } else {
            self.bin.center()
        };
        if let Some(tower) = group.iter().find(|&&i| arena[i].is_wipe_tower) {
            start = arena[*tower].bounding_box().center();
        }
        self.objective = Objective::Fixed { start };
    }

    /// Takes an item placed by `pack` onto the bed (`accept`).
    pub fn accept(&mut self, result: &PackResult, index: usize, arena: &mut [Item]) {
        if !result.packed {
            return;
        }
        let item = &mut arena[index];
        item.set_translation(result.translation);
        item.set_rotation(result.rotation);
        self.items.push(index);
        let mut pile = self.merged_pile.clone();
        pile.push(ExPolygon {
            contour: item.transformed(),
            holes: Vec::new(),
        });
        self.merged_pile = edge_clip::union_ex(&pile).unwrap_or(pile);
        self.score += result.score;
    }

    /// Lines the pile up and takes away the items `clear` picks
    /// (`clearItems`).
    pub fn clear_items(
        &mut self,
        arena: &mut [Item],
        config: &PlacerConfig,
        clear: impl Fn(&Item) -> bool,
    ) {
        self.final_align(arena, config);
        self.items.retain(|&i| !clear(&arena[i]));
    }

    fn place_outside_of_bin(&self, item: &mut Item) {
        let bb = item.bounding_box();
        let v = Point2 {
            x: bb.max.x,
            y: bb.min.y,
        };
        item.translate(Point2 {
            x: self.bin.max.x - v.x,
            y: self.bin.max.y - v.y,
        });
    }

    fn set_initial_position(&self, item: &mut Item, config: &PlacerConfig) {
        let bb = item.bounding_box();
        let shrink = Point2 { x: 10, y: 10 };
        let bin = Bbox::new(add(self.bin.min, shrink), sub(self.bin.max, shrink));
        let (ci, cb) = match config.starting_point {
            Align::Center => (bb.center(), bin.center()),
            Align::BottomLeft => (bb.min, bin.min),
            Align::TopRight => (bb.max, bin.max),
            _ => return,
        };
        item.translate(sub(cb, ci));
    }

    fn free_places(&self, item: &Item, arena: &[Item]) -> Vec<ExPolygon> {
        let stationary: Vec<Polygon> = self.items.iter().map(|&i| arena[i].transformed()).collect();
        free_region(&item.transformed(), &stationary, &self.bin)
    }

    /// Tries an item on this bed (`_trypack`). The item is left where the
    /// search left it, as the reference leaves it.
    pub fn pack(
        &self,
        item: &mut Item,
        arena: &[Item],
        remaining: usize,
        config: &PlacerConfig,
        scorer: &impl Scorer,
    ) -> PackResult {
        let mut can_pack = false;
        let mut best_overfit = f64::MAX;
        let mut global_score = f64::MAX;
        let initial_tr = item.translation();
        let initial_rot = item.rotation();
        let mut final_tr = Point2::default();
        let mut final_rot = initial_rot;

        let first_object = self
            .items
            .iter()
            .all(|&i| arena[i].is_virtual && !arena[i].is_wipe_tower);
        let overlap_with_virtual = |item: &Item| -> f64 {
            if self.items.is_empty() {
                return 0.0;
            }
            let v = item.reference_vertex();
            let clear = self
                .free_places(item, arena)
                .iter()
                .any(|region| region_side(region, v) != Side::Outside);
            if clear { 0.0 } else { 1.0 }
        };

        if first_object {
            self.set_initial_position(item, config);
            let (mut best_tr, mut best_rot) = (item.translation(), item.rotation());
            best_overfit = overfit(&item.bounding_box(), &self.bin) + overlap_with_virtual(item);
            for rot in &config.rotations {
                item.set_translation(initial_tr);
                item.set_rotation(initial_rot + rot);
                self.set_initial_position(item, config);
                let of = overfit(&item.bounding_box(), &self.bin);
                if of + overlap_with_virtual(item) < best_overfit {
                    best_overfit = of;
                    best_tr = item.translation();
                    best_rot = item.rotation();
                }
            }
            can_pack = best_overfit <= 0.0;
            if can_pack {
                global_score = 0.2;
            }
            item.set_rotation(best_rot);
            item.set_translation(best_tr);
        }

        if !can_pack {
            let pile = self.pile(arena, remaining, scorer);
            for rot in &config.rotations {
                item.set_translation(initial_tr);
                item.set_rotation(initial_rot + rot);
                self.place_outside_of_bin(item);
                let nfps = self.free_places(item, arena);
                let iv = item.reference_vertex();
                let startpos = item.translation();
                let caches: Vec<EdgeCache> = nfps.iter().map(EdgeCache::of).collect();
                let at = |nfp: usize, ring: usize, pos: f64| {
                    add(sub(caches[nfp].point(ring, pos), iv), startpos)
                };
                let mut best_score = f64::MAX;
                let mut optimum: Option<(usize, usize, f64)> = None;
                let budget = (1000.0 * config.accuracy).floor() as usize;
                for (ch, cache) in caches.iter().enumerate() {
                    for ring in 0..cache.ring_count() {
                        let mut best_here: Option<subplex::Optimum> = None;
                        for start in cache.corners(ring) {
                            let mut probe = item.clone();
                            let result = subplex::minimize(
                                |pos| {
                                    probe.set_translation(at(ch, ring, pos));
                                    scorer.score(&probe, &pile, &self.objective, &self.bin)
                                },
                                start,
                                0.0,
                                1.0,
                                budget,
                                1e-20,
                            );
                            if best_here.is_none_or(|b| result.score < b.score) {
                                best_here = Some(result);
                            }
                        }
                        let Some(found) = best_here else { continue };
                        if found.score < best_score {
                            // The pile with it must still fit the bed.
                            item.set_translation(at(ch, ring, found.x));
                            let mut points: Vec<Point2> = self
                                .merged_pile
                                .iter()
                                .flat_map(|r| r.contour.points.iter().copied())
                                .collect();
                            points.extend(item.transformed().points);
                            let hull = convex_hull(points);
                            let miss = if config.alignment == Align::Unaligned {
                                // `isInside(chull, bin)`.
                                let b = Bbox::of(&hull.points);
                                if b.min.x >= self.bin.min.x
                                    && b.min.y >= self.bin.min.y
                                    && b.max.x <= self.bin.max.x
                                    && b.max.y <= self.bin.max.y
                                {
                                    -1.0
                                } else {
                                    1.0
                                }
                            } else {
                                overfit(&Bbox::of(&hull.points), &self.bin)
                            };
                            if miss <= 0.0 {
                                best_score = found.score;
                                optimum = Some((ch, ring, found.x));
                            } else {
                                best_overfit = best_overfit.min(miss);
                            }
                        }
                    }
                }
                if let Some((ch, ring, pos)) = optimum
                    && best_score < global_score
                {
                    final_tr = at(ch, ring, pos);
                    final_rot = initial_rot + rot;
                    can_pack = true;
                    global_score = best_score;
                }
            }
            item.set_translation(final_tr);
            item.set_rotation(final_rot);
        }

        if can_pack {
            PackResult {
                packed: true,
                translation: item.translation(),
                rotation: item.rotation(),
                score: global_score,
            }
        } else {
            PackResult::failed()
        }
    }

    /// The bed as the objective sees it (`before_packing`).
    fn pile<'a>(&'a self, arena: &'a [Item], remaining: usize, scorer: &impl Scorer) -> Pile<'a> {
        let mut big = Vec::new();
        let mut all = Vec::new();
        for &i in &self.items {
            let item = &arena[i];
            if item.is_virtual {
                continue;
            }
            if scorer.is_big(item.area()) {
                big.push((item.bounding_box(), i));
            }
            all.push((item.bounding_box(), i));
        }
        Pile {
            arena,
            items: &self.items,
            pile_box: Bbox::of_regions(&self.merged_pile),
            big,
            all,
            remaining,
        }
    }

    /// Puts the finished pile where the configuration says, clear of the
    /// fixed items (`finalAlign`).
    ///
    /// The reference counts the fixed objects (not the virtual ones) in the
    /// pile's box and moves them with it (nfpplacer.hpp l.1075, l.1170);
    /// the moves of fixed items are thrown away afterwards, so it lines up
    /// a box that holds objects which stay where they are. Here the box is
    /// the box of what was placed, and only that is moved.
    pub fn final_align(&self, arena: &mut [Item], config: &PlacerConfig) {
        if self.items.is_empty() || config.alignment == Align::Unaligned {
            return;
        }
        let movable: Vec<usize> = self
            .items
            .iter()
            .copied()
            .filter(|&i| !arena[i].is_virtual && !arena[i].fixed)
            .collect();
        let bb = movable
            .iter()
            .fold(Bbox::default(), |bb, &i| bb.merge(arena[i].bounding_box()));
        let bin = self.bin;
        let (ci, cb) = match config.alignment {
            Align::Center => (bb.center(), bin.center()),
            Align::BottomLeft => (bb.min, bin.min),
            Align::TopRight => (bb.max, bin.max),
            Align::UserDefined => (bb.center(), config.best_object_pos),
            Align::Unaligned => return,
        };
        let mut d = sub(cb, ci);
        let clamp = |v: i64, lo: i64, hi: i64| {
            if lo > hi {
                // A pile bigger than the bed: in its middle.
                (lo + hi) / 2
            } else {
                v.clamp(lo, hi)
            }
        };
        d.x = clamp(d.x, bin.min.x - bb.min.x, bin.max.x - bb.max.x);
        d.y = clamp(d.y, bin.min.y - bb.min.y, bin.max.y - bb.max.y);
        if movable.is_empty() {
            return;
        }
        let hull = convex_hull(movable.iter().flat_map(|&i| arena[i].transformed().points));
        let excludes: Vec<Polygon> = self
            .items
            .iter()
            .filter(|&&i| arena[i].fixed)
            .map(|&i| arena[i].transformed())
            .collect();
        let nfps = free_region(&hull, &excludes, &bin);
        if nfps.is_empty() {
            return;
        }
        let reference = add(edge_clip::reference_vertex(&hull).unwrap_or_default(), d);
        let inside = nfps
            .iter()
            .any(|nfp| region_side(nfp, reference) == Side::Inside);
        if !inside {
            let projected = projection_onto(&nfps, reference);
            d = add(d, sub(projected, reference));
        }
        for i in movable {
            arena[i].translate(d);
        }
    }
}
