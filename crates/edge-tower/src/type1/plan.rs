//! The Type 1 tower's planner (WipeTower.cpp at the pin): which block
//! each tool change purges into, how deep each block and each layer is,
//! where every purge starts and the wall is left open for it, and where
//! the tower stands.
//!
//! Ported: `plan_toolchange` / `replan_toolchange`, `plan_tower_new`,
//! `calc_block_infill_gap`, `generate_wipe_tower_blocks`, the stability
//! table (`min_depth_per_height`), `update_all_layer_depth`,
//! `layer_y_shifts`, `get_all_wall_skip_points`, `set_rib_offset`,
//! `get_auto_brim_by_height`, and the footprint estimates
//! (`estimate_tower_blocks_depth` and its company).
//!
//! For a printer of one nozzle only. Refused rather than planned some
//! other way: a printer of more than one nozzle, or whose rack hotends
//! count as extruders (the nozzle-change ramming, grouping, and the H3C's
//! ladder and merged slabs, `plan_tower_ladder` / `plan_merged_slabs`);
//! sparse layers laid as one (`combine_sparse_wipe_tower_plan`); and the
//! tower interfaces (`mark_interfaces`).

use std::collections::BTreeMap;
use std::f32::consts::PI;

use edge_clip::{ClipError, Join};
use edge_geom::{ExPolygon, scaled, unscaled};

use super::settings::{Settings, WIDTH_TO_NOZZLE, Wall};

/// libslic3r's `EPSILON` and the tower's own `WT_EPSILON`.
const EPSILON: f32 = 1e-4;
const WT_EPSILON: f32 = 1e-3;

/// What can stop the planner.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    /// Something the reference plans that is not done here yet.
    #[error("Bambu's prime tower: {0}")]
    Unsupported(String),
    /// A tower no row fits across: the reference plans no change into it
    /// and so lays no tower, while the printer still changes filament.
    #[error(
        "Bambu's prime tower: {width} mm wide leaves no room inside its wall for a purge line \
         {line} mm wide"
    )]
    TooNarrow { width: f32, line: f32 },
    #[error(transparent)]
    Clip(#[from] ClipError),
}

/// One tool change asked of the tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChangeIn {
    pub old: usize,
    pub new: usize,
    /// What the change G-code flushes, in mm³ (Print.cpp `volume_to_purge`):
    /// carried for the G-code, not laid on the tower.
    pub purge_volume: f32,
}

/// One layer the tower stands on, with its changes in the order made.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerIn {
    /// Its top and how thick it is (`wipe_tower_print_z`,
    /// `wipe_tower_layer_height`).
    pub z: f32,
    pub height: f32,
    pub changes: Vec<ChangeIn>,
}

/// A tool change as planned (`WipeTowerInfo::ToolChange`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub old: usize,
    pub new: usize,
    /// What the new filament lays on the tower, in mm³, and as a length of
    /// row (`wipe_volume`, `wipe_length`).
    pub wipe_volume: f32,
    pub wipe_length: f32,
    pub purge_volume: f32,
    /// The depth its purge takes in its block (`required_depth`).
    pub required_depth: f32,
    /// Where its purge starts on the outer wall, in the tower's frame
    /// (`start_point`; set with the wall gap).
    pub start_point: (f32, f32),
}

/// A layer as planned (`WipeTowerInfo`).
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub z: f32,
    pub height: f32,
    /// What it and every layer above need of the tower's depth.
    pub depth: f32,
    /// How far apart its rows are, as a share of the block gap.
    pub extra_spacing: f32,
    pub changes: Vec<Change>,
}

/// How a block is laid on a layer (`WipeTowerLayerType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerType {
    Normal,
    /// The first layer where the category starts or stops being purged:
    /// solid and slow.
    Contact,
    /// A layer below such a change: solid.
    Solid,
    /// The layer above a contact layer: solid and slow.
    ContactUp,
}

/// The purges of one adhesiveness category, in a band of the tower
/// (`WipeTowerBlock`).
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub id: usize,
    pub category: i32,
    pub layer_depths: Vec<f32>,
    pub layer_types: Vec<LayerType>,
    pub layer_heights: Vec<f32>,
    pub depth: f32,
    pub start_depth: f32,
}

/// The ribs once planned (`m_rib_width`, `m_rib_length`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ribs {
    pub width: f32,
    pub length: f32,
    pub extra: f32,
    pub fillet: bool,
}

/// A box in the tower's frame (`box_coordinates`): left, bottom, right,
/// top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
    pub top: f32,
}

impl Box2 {
    pub(crate) fn new(left: f32, bottom: f32, width: f32, height: f32) -> Self {
        Self {
            left,
            bottom,
            right: left + width,
            top: bottom + height,
        }
    }

    fn expand(&self, by: f32) -> Self {
        Self {
            left: self.left - by,
            bottom: self.bottom - by,
            right: self.right + by,
            top: self.top + by,
        }
    }
}

/// The tower as planned.
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub layers: Vec<Layer>,
    pub blocks: Vec<Block>,
    /// The tower's width (squared with ribs) and depth
    /// (`m_wipe_tower_width`, `m_wipe_tower_depth`), its height
    /// (`m_wipe_tower_height`, the last layer's top), and its line width.
    pub width: f32,
    pub depth: f32,
    pub height: f32,
    pub line_width: f32,
    /// What the stability minimum stretches the rows by (`m_extra_spacing`
    /// after planning).
    pub extra_spacing: f32,
    /// The block row gap of each category (`m_block_infill_gap_width`
    /// first: purge rows; second: ramming rows).
    pub gaps: BTreeMap<i32, (f32, f32)>,
    /// The brim width set or worked out from the height ("auto").
    pub brim_width: f32,
    pub ribs: Option<Ribs>,
    /// The tower's first layer (`m_first_layer_idx`), and the filament in
    /// hand before its first change (`m_current_tool`).
    pub first_layer: usize,
    pub start_tool: usize,
    /// How far up each layer's frame is laid (`layer_y_shifts`).
    pub y_shifts: Vec<f32>,
    /// Where each layer's wall is left open (`m_wall_skip_points`).
    pub skip_points: Vec<Vec<(f32, f32)>>,
    /// With ribs: how far the tower's frame is moved so that the box round
    /// its first layer's wall starts at the place set, and that box's size
    /// (`m_rib_offset`, `m_rib_footprint`).
    pub rib_offset: (f32, f32),
    pub rib_footprint: (f32, f32),
    /// Whether the layers with no tool change are left out.
    pub skip_sparse: bool,
    /// The place set (`m_wipe_tower_pos`).
    pub origin: (f32, f32),
}

/// The least depth a tower `height` mm tall should have to stand
/// (WipeTower.cpp `get_limit_depth_by_height` with `min_depth_per_height`,
/// Bambu Studio 2.9.1's table): 5 mm up to 5 mm tall, then linearly to 20
/// at 100, 40 at 250 and 60 at 350, and 60 above.
pub fn limit_depth(height: f32) -> f32 {
    const TABLE: [(f32, f32); 4] = [(5.0, 5.0), (100.0, 20.0), (250.0, 40.0), (350.0, 60.0)];
    for (index, &(at, depth)) in TABLE.iter().enumerate() {
        if at >= height {
            return depth;
        }
        let Some(&(next_at, next_depth)) = TABLE.get(index + 1) else {
            return depth;
        };
        if next_at > height {
            return depth + (height - at) / (next_at - at) * (next_depth - depth);
        }
    }
    TABLE[TABLE.len() - 1].1
}

/// The brim "auto" gives a tower so tall (`get_auto_brim_by_height`).
pub fn auto_brim(height: f32) -> f32 {
    if height < 100.0 {
        height / 100.0 * 8.0
    } else {
        8.0
    }
}

fn align_round(value: f32, base: f32) -> f32 {
    (value / base).round() * base
}

fn align_ceil(value: f32, base: f32) -> f32 {
    (value / base).ceil() * base
}

fn align_floor(value: f32, base: f32) -> f32 {
    (value / base).floor() * base
}

/// How long a line `width` wide on a layer `height` thick lays `volume`
/// mm³ (`volume_to_length`), and the other way (`length_to_volume`).
fn volume_to_length(volume: f32, width: f32, height: f32) -> f32 {
    (volume / (height * (width - height * (1.0 - PI / 4.0)))).max(0.0)
}

fn length_to_volume(length: f32, width: f32, height: f32) -> f32 {
    (length * (height * (width - height * (1.0 - PI / 4.0)))).max(0.0)
}

/// The width of the lines a nozzle-change ram lays
/// (`nozzle_change_perimeter_width`).
fn nozzle_change_width(nozzle: f32) -> f32 {
    const TABLE: [(f32, f32); 4] = [(0.2, 0.5), (0.4, 1.0), (0.6, 1.2), (0.8, 1.4)];
    TABLE
        .iter()
        .find(|(diameter, _)| *diameter == nozzle)
        .map_or(2.0 * nozzle * WIDTH_TO_NOZZLE, |(_, width)| *width)
}

/// One filament's share of a tower layer, as the footprint estimate takes
/// it (WipeTower.hpp `PurgeEstimate`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PurgeEstimate {
    pub prime_volume: f32,
    pub category: i32,
    /// mm of filament rammed when it leaves its nozzle; 0 for none.
    pub filament_change_length: f32,
    pub filament_diameter: f32,
    /// On a plate of nozzles of more than one size: its nozzle, its line
    /// width and its slab; 0 for the tower's.
    pub nozzle_diameter: f32,
    pub line_width: f32,
    pub slab_height: f32,
}

impl Default for PurgeEstimate {
    fn default() -> Self {
        Self {
            prime_volume: 0.0,
            category: 0,
            filament_change_length: 0.0,
            filament_diameter: 1.75,
            nozzle_diameter: 0.0,
            line_width: 0.0,
            slab_height: 0.0,
        }
    }
}

/// The depth of the Type 1 purge stack at `width` before the tower is
/// sliced (`estimate_tower_blocks_depth`): whole lines at the block gap,
/// one block per category as deep as its worst layer, behind one line
/// width. With `max_height` at or above 0, a rectangle tower that tall,
/// whose blocks with ramming may lay their purge rows at the ramming
/// pitch.
#[allow(clippy::too_many_arguments)]
pub fn estimate_tower_blocks_depth(
    purges: &[PurgeEstimate],
    width: f32,
    layer_height: f32,
    nozzle_diameter: f32,
    extra_spacing: f32,
    tower_line_width: f32,
    max_height: f32,
) -> f32 {
    if purges.is_empty() || layer_height < EPSILON || nozzle_diameter < EPSILON {
        return 0.0;
    }
    let pw = if tower_line_width > 0.0 {
        tower_line_width
    } else {
        nozzle_diameter * WIDTH_TO_NOZZLE
    };
    let ncpw = nozzle_change_width(nozzle_diameter);
    let line_width = width - 2.0 * pw;
    if line_width <= EPSILON {
        return 0.0;
    }
    let line_area = |w: f32| layer_height * (w - layer_height * (1.0 - PI / 4.0));
    let extra_width = (extra_spacing - 1.0) * pw;
    let gap = pw + extra_width;
    let nc_gap = ncpw + extra_width;
    let key_of = |p: &PurgeEstimate| {
        if p.nozzle_diameter > 0.0 {
            (p.category << 10) | (((p.nozzle_diameter / 0.05 + 0.5) as i32) << 4)
        } else {
            p.category
        }
    };
    let pw_of = |p: &PurgeEstimate| if p.line_width > 0.0 { p.line_width } else { pw };
    let ncpw_of = |p: &PurgeEstimate| {
        if p.nozzle_diameter > 0.0 {
            nozzle_change_width(p.nozzle_diameter)
        } else {
            ncpw
        }
    };
    let area_of = |p: &PurgeEstimate, w: f32| {
        let h = if p.slab_height > 0.0 {
            p.slab_height
        } else {
            layer_height
        };
        h * (w - h * (1.0 - PI / 4.0))
    };
    let per_purge = purges.iter().any(|p| p.nozzle_diameter > 0.0);
    #[derive(Default)]
    struct Stack {
        depth: f32,
        min_purge: f32,
        filaments: usize,
        ramming: bool,
    }
    // A layer purges into at most (filaments - 1) targets, so a category
    // holding every filament never sees its smallest purge in its worst
    // layer.
    let stack = |purge_gap: &dyn Fn(i32) -> f32,
                 ramming_gap: &dyn Fn(i32) -> f32,
                 blocks: &mut BTreeMap<i32, Stack>| {
        for purge in purges {
            let key = key_of(purge);
            let block = blocks.entry(key).or_default();
            let purge_depth = if per_purge {
                (purge.prime_volume / area_of(purge, pw_of(purge)) / line_width).ceil()
                    * (purge_gap(key) + pw_of(purge) - pw)
            } else {
                (purge.prime_volume / line_area(pw) / line_width).ceil() * purge_gap(purge.category)
            };
            block.min_purge = if block.filaments == 0 {
                purge_depth
            } else {
                block.min_purge.min(purge_depth)
            };
            block.depth += purge_depth;
            block.filaments += 1;
            if purge.filament_change_length > EPSILON {
                let filament_area = PI * purge.filament_diameter * purge.filament_diameter / 4.0;
                if per_purge {
                    let own = ncpw_of(purge);
                    let length = purge.filament_change_length * filament_area / area_of(purge, own);
                    block.depth += (length / (width - own - pw_of(purge))).ceil()
                        * (ramming_gap(key) + own - ncpw);
                } else {
                    let length = purge.filament_change_length * filament_area / line_area(ncpw);
                    block.depth +=
                        (length / (width - ncpw - pw)).ceil() * ramming_gap(purge.category);
                }
                block.ramming = true;
            }
        }
        blocks
            .values()
            .map(|block| {
                if block.filaments == purges.len() {
                    block.depth - block.min_purge
                } else {
                    block.depth
                }
            })
            .sum::<f32>()
    };
    let mut raw_blocks = BTreeMap::new();
    let raw_depth = stack(&|_| pw, &|_| ncpw, &mut raw_blocks);
    let mut aligned = Vec::new();
    if max_height >= 0.0 {
        let height_to_depth = limit_depth(max_height);
        let height_to_spacing = if raw_depth > height_to_depth || raw_depth < EPSILON {
            1.0
        } else {
            height_to_depth / raw_depth
        };
        for (category, block) in &raw_blocks {
            if block.ramming && nc_gap * height_to_spacing >= 2.0 * ncpw {
                aligned.push(*category);
            }
        }
    }
    let mut blocks = BTreeMap::new();
    pw + stack(
        &|category| {
            if aligned.contains(&category) {
                nc_gap
            } else {
                gap
            }
        },
        &|_| nc_gap,
        &mut blocks,
    )
}

/// The side of the square round a ribbed tower's first layer, brim left
/// out (`rib_footprint_side`, Type 1's table).
pub fn rib_footprint_side(
    width: f32,
    depth: f32,
    rib_width: f32,
    extra_rib_length: f32,
    max_height: f32,
) -> f32 {
    if width < EPSILON || depth < EPSILON {
        return 0.0;
    }
    let diagonal = (width * width + depth * depth).sqrt();
    let mut rib_length = diagonal;
    let min_depth = limit_depth(max_height);
    if depth + EPSILON < min_depth {
        rib_length = rib_length.max(min_depth * 2f32.sqrt());
    }
    rib_length = diagonal.max(rib_length + extra_rib_length);
    let rib_w = rib_width.min(width.min(depth) / 2.0);
    let per_side = ((rib_length - diagonal) / 2.0 + rib_w / 2.0) / 2f32.sqrt();
    width.max(depth) + 2.0 * per_side
}

/// A ribbed tower squared from the depth at the width set, then re-planned
/// at the square's width (`estimate_rib_tower_bbox_side`).
#[allow(clippy::too_many_arguments)]
pub fn estimate_rib_tower_bbox_side(
    purges: &[PurgeEstimate],
    width: f32,
    layer_height: f32,
    nozzle_diameter: f32,
    extra_spacing: f32,
    rib_width: f32,
    extra_rib_length: f32,
    max_height: f32,
    line_width: f32,
) -> f32 {
    if purges.is_empty() || width < EPSILON || layer_height < EPSILON || nozzle_diameter < EPSILON {
        return 0.0;
    }
    let pw = if line_width > 0.0 {
        line_width
    } else {
        nozzle_diameter * WIDTH_TO_NOZZLE
    };
    let depth_at = |width: f32| {
        estimate_tower_blocks_depth(
            purges,
            width,
            layer_height,
            nozzle_diameter,
            extra_spacing,
            line_width,
            -1.0,
        )
    };
    let square = align_ceil((depth_at(width) * width).sqrt(), pw);
    rib_footprint_side(
        square,
        depth_at(square),
        rib_width,
        extra_rib_length,
        max_height,
    )
}

/// The brim the tower lays, as wide as it comes out in whole loops with
/// half a spacing of line added (`estimate_brim_real_width`, Type 1).
pub fn estimate_brim_real_width(
    brim_width: f32,
    nozzle_diameter: f32,
    first_layer_height: f32,
    line_width: f32,
) -> f32 {
    if brim_width <= 0.0 {
        return brim_width;
    }
    let spacing = (if line_width > 0.0 {
        line_width
    } else {
        nozzle_diameter * WIDTH_TO_NOZZLE
    }) - first_layer_height * (1.0 - PI / 4.0);
    if spacing <= EPSILON {
        return brim_width;
    }
    let loops = ((brim_width + spacing / 2.0) / spacing) as i32;
    loops as f32 * spacing + spacing / 2.0
}

/// The planner's working state: WipeTower's members it uses.
struct Planner<'a> {
    settings: &'a Settings,
    plan: Vec<Layer>,
    line: f32,
    width: f32,
    depth: f32,
    height: f32,
    extra_spacing: f32,
    gaps: BTreeMap<i32, (f32, f32)>,
    blocks: Vec<Block>,
    first_layer: Option<usize>,
    current_tool: usize,
    rib_length: f32,
}

impl Planner<'_> {
    fn category(&self, filament: usize) -> i32 {
        self.settings.category(filament)
    }

    /// The row gap of a filament's block (`get_block_gap_width`, purge
    /// rows): a block not planned yet lays its rows a line apart.
    fn gap_width(&self, filament: usize) -> f32 {
        self.gaps
            .get(&self.category(filament))
            .map_or(self.line, |gap| gap.0)
    }

    /// `replan_toolchange`, one nozzle: whole rows of the new filament's
    /// prime at its block's gap.
    fn replan(&self, change: &mut Change, height: f32) {
        let across = self.width - 2.0 * self.line;
        if across <= EPSILON {
            return;
        }
        change.wipe_length = volume_to_length(change.wipe_volume, self.line, height);
        change.required_depth = (change.wipe_length / across).ceil() * self.gap_width(change.new);
    }

    fn replan_all(&mut self) {
        let mut plan = std::mem::take(&mut self.plan);
        for layer in &mut plan {
            for change in &mut layer.changes {
                self.replan(change, layer.height);
            }
        }
        self.plan = plan;
    }

    /// `calc_block_infill_gap`, one nozzle: no block rams, so every block's
    /// rows are `prime_tower_infill_gap` apart. Then every change is
    /// re-planned at its block's gap.
    fn calc_block_infill_gap(&mut self) {
        // Each category the layers purge into, top down (its depth would
        // decide a ramming block's gap; there are none on one nozzle).
        let mut categories = BTreeMap::new();
        for layer in &self.plan {
            for change in &layer.changes {
                categories.insert(self.category(change.new), ());
            }
        }
        let extra_width = (self.extra_spacing - 1.0) * self.line;
        self.gaps = categories
            .into_keys()
            .map(|category| {
                let gap = extra_width + self.line;
                (category, (gap, gap))
            })
            .collect();
        self.replan_all();
        self.extra_spacing = 1.0;
    }

    /// `generate_wipe_tower_blocks`: one block per category, as deep on a
    /// layer as on any above it; with `types`, how each block is laid on
    /// each layer.
    fn generate_blocks(&mut self, types: bool) {
        let count = self.plan.len();
        // 1. Each layer's depth per category, in the order they come.
        let layer_depths: Vec<Vec<(i32, f32)>> = self
            .plan
            .iter()
            .map(|layer| {
                let mut depths: Vec<(i32, f32)> = Vec::new();
                for change in &layer.changes {
                    let category = self.category(change.new);
                    match depths.iter_mut().find(|(at, _)| *at == category) {
                        Some((_, depth)) => *depth += change.required_depth,
                        None => depths.push((category, change.required_depth)),
                    }
                }
                depths
            })
            .collect();
        let depth_of = |layer: usize, category: i32| {
            layer_depths[layer]
                .iter()
                .find(|(at, _)| *at == category)
                .map_or(0.0, |(_, depth)| *depth)
        };
        // 2. The blocks, in the order their categories first come.
        self.blocks.clear();
        for (layer, depths) in layer_depths.iter().enumerate() {
            for &(category, depth) in depths {
                let at = match self.blocks.iter().position(|b| b.category == category) {
                    Some(at) => at,
                    None => {
                        self.blocks.push(Block {
                            id: self.blocks.len(),
                            category,
                            layer_depths: vec![0.0; count],
                            layer_types: vec![LayerType::Normal; count],
                            layer_heights: vec![0.0; count],
                            depth: 0.0,
                            start_depth: 0.0,
                        });
                        self.blocks.len() - 1
                    }
                };
                let block = &mut self.blocks[at];
                block.depth = block.depth.max(depth);
                block.layer_depths[layer] = depth;
            }
        }
        // 3. A block is as deep on a layer as on any layer above it.
        for layer in (0..count).rev() {
            self.plan[layer].depth = 0.0;
            for block in &mut self.blocks {
                if layer + 1 < count {
                    block.layer_depths[layer] =
                        block.layer_depths[layer].max(block.layer_depths[layer + 1]);
                }
                self.plan[layer].depth += block.layer_depths[layer];
            }
        }
        // `prime_tower_enable_framework`: every layer keeps the first's.
        if self.settings.framework {
            for layer in 1..count {
                self.plan[layer].depth = 0.0;
                for block in &mut self.blocks {
                    block.layer_depths[layer] = block.layer_depths[0];
                    self.plan[layer].depth += block.layer_depths[layer];
                }
            }
        }
        if types {
            // 4. The categories each layer purges: a layer without changes
            // is laid by the filament in hand.
            let mut loaded = self
                .plan
                .iter()
                .find_map(|layer| layer.changes.first().map(|change| change.old))
                .unwrap_or(self.current_tool);
            let used: Vec<Vec<i32>> = self
                .plan
                .iter()
                .map(|layer| {
                    let mut used = Vec::new();
                    if let Some(first) = layer.changes.first() {
                        loaded = first.old;
                    }
                    if layer.changes.is_empty() {
                        used.push(self.category(loaded));
                    }
                    for change in &layer.changes {
                        used.push(self.category(change.old));
                        used.push(self.category(change.new));
                    }
                    if let Some(last) = layer.changes.last() {
                        loaded = last.new;
                    }
                    used
                })
                .collect();
            const SOLID_BELOW: usize = 4;
            for block in &mut self.blocks {
                let has = |layer: usize| used[layer].contains(&block.category);
                for layer in 0..count {
                    // Solid: not filled by purges here, and the category
                    // changes within the next layers.
                    if depth_of(layer, block.category) < block.layer_depths[layer] - self.line {
                        let here = has(layer);
                        for ahead in (1..=SOLID_BELOW).rev() {
                            if layer + ahead < count && here != has(layer + ahead) {
                                block.layer_types[layer] = LayerType::Solid;
                                break;
                            }
                        }
                    }
                    if layer > 0 {
                        if has(layer) != has(layer - 1) {
                            block.layer_types[layer] = LayerType::Contact;
                        }
                        if block.layer_types[layer - 1] == LayerType::Contact
                            && block.layer_types[layer] != LayerType::Contact
                        {
                            block.layer_types[layer] = LayerType::ContactUp;
                        }
                    }
                }
            }
        }
        // Each band lays the layer's own height (no merged slabs here).
        for block in &mut self.blocks {
            for (layer, height) in block.layer_heights.iter_mut().enumerate() {
                *height = self.plan[layer].height;
            }
        }
    }

    /// `update_all_layer_depth`: the blocks stretched to the stability
    /// minimum and stacked from one line width in.
    fn update_all_layer_depth(&mut self, idle_depth: f32) {
        self.depth = 0.0;
        let start_offset = self.line;
        let mut start = start_offset;
        for block in &mut self.blocks {
            block.depth *= self.extra_spacing;
            block.start_depth = start;
            start += block.depth;
            self.depth += block.depth;
            for depth in &mut block.layer_depths {
                *depth *= self.extra_spacing;
            }
        }
        for layer in &mut self.plan {
            layer.depth *= self.extra_spacing;
        }
        if self.depth > 0.0 {
            self.depth += start_offset;
        }
        // A smooth timelapse lays the tower whole on every layer.
        if self.settings.smooth_timelapse {
            if self.depth.abs() < EPSILON {
                self.depth = idle_depth;
            }
            for layer in &mut self.plan {
                layer.depth = self.depth;
            }
        }
    }

    /// `layer_y_shifts`: a single block narrows with the layers above, and
    /// is laid in the middle of the tower, to the row spacing.
    fn y_shifts(&self) -> Vec<f32> {
        let mut shift = 0.0;
        self.plan
            .iter()
            .map(|layer| {
                if self.blocks.len() == 1
                    && layer.depth >= self.line
                    && layer.depth < self.depth - self.line
                {
                    shift = align_round(
                        (self.depth - layer.depth) / 2.0,
                        self.extra_spacing * self.line,
                    );
                }
                shift
            })
            .collect()
    }

    /// `get_all_wall_skip_points` / `get_wall_skip_points`, one nozzle:
    /// each purge starts on the outer wall, by layer at the left front,
    /// the right back, the right front or the left back of its rows, and
    /// the wall is left open there.
    fn wall_skip_points(&mut self) -> Vec<Vec<(f32, f32)>> {
        let mut points = vec![Vec::new(); self.plan.len()];
        if !self.settings.wall_gap {
            return points;
        }
        for (index, layer) in self.plan.iter_mut().enumerate() {
            let mut reached: Vec<(i32, f32)> = Vec::new();
            for change in &mut layer.changes {
                let category = self.settings.category(change.new);
                let Some(block) = self.blocks.iter().find(|b| b.category == category) else {
                    continue;
                };
                let at = match reached.iter().position(|(c, _)| *c == category) {
                    Some(at) => at,
                    None => {
                        reached.push((category, block.start_depth));
                        reached.len() - 1
                    }
                };
                let process = reached[at].1;
                let wipe_depth = change.required_depth;
                let gap = self.gaps.get(&category).map_or(self.line, |gap| gap.0);
                let last_row = process + wipe_depth - layer.extra_spacing * gap;
                let point = match index % 4 {
                    0 => (0.0, process),
                    1 => (self.width, last_row),
                    2 => (self.width, process),
                    _ => (0.0, last_row),
                };
                change.start_point = point;
                points[index].push(point);
                reached[at].1 = process + wipe_depth;
            }
        }
        points
    }
}

/// Plans the tower for `layers`, the layers it stands on from the print's
/// first up to its last tool change, with `start_tool` the filament the
/// print starts with (WipeTower.cpp `plan_toolchange` for each, then
/// `generate` up to the layers' laying: `plan_tower_new`, `set_rib_offset`).
pub fn plan(settings: &Settings, layers: &[LayerIn], start_tool: usize) -> Result<Plan, PlanError> {
    if settings.nozzles > 1 || settings.hotend_as_extruder {
        return Err(PlanError::Unsupported(
            "printers of more than one nozzle, or of rack hotends, are not planned yet (nozzle \
             grouping, ramming on a nozzle change, the H3C's merged slabs)"
                .to_owned(),
        ));
    }
    if settings.combine_sparse {
        return Err(PlanError::Unsupported(
            "wipe_tower_sparse_layers_combination: laying the layers with no tool change as one \
             is not planned yet"
                .to_owned(),
        ));
    }
    if settings.edge_interface {
        return Err(PlanError::Unsupported(
            "the tower interface options (wipe_tower_interface_temp, _run_in, _extra_prime) are \
             not planned yet"
                .to_owned(),
        ));
    }
    let line = settings.line_width;
    let mut planner = Planner {
        settings,
        plan: Vec::new(),
        line,
        width: settings.width,
        depth: 0.0,
        height: 0.0,
        extra_spacing: settings.infill_gap,
        gaps: BTreeMap::new(),
        blocks: Vec::new(),
        first_layer: None,
        current_tool: start_tool,
        rib_length: 0.0,
    };
    // `plan_toolchange`.
    for layer in layers {
        if planner
            .plan
            .last()
            .is_none_or(|last| last.z + WT_EPSILON < layer.z)
        {
            planner.plan.push(Layer {
                z: layer.z,
                height: layer.height,
                depth: 0.0,
                extra_spacing: 1.0,
                changes: Vec::new(),
            });
        }
        let index = planner.plan.len() - 1;
        if planner.first_layer.is_none() && (!settings.skip_sparse || !layer.changes.is_empty()) {
            planner.first_layer = Some(index);
        }
        for asked in &layer.changes {
            if asked.old == asked.new {
                continue;
            }
            if planner.width - 2.0 * line <= EPSILON {
                return Err(PlanError::TooNarrow {
                    width: planner.width,
                    line,
                });
            }
            let mut change = Change {
                old: asked.old,
                new: asked.new,
                wipe_volume: settings.prime_volume(asked.new, false),
                wipe_length: 0.0,
                purge_volume: asked.purge_volume,
                required_depth: 0.0,
                start_point: (0.0, 0.0),
            };
            planner.replan(&mut change, layer.height);
            planner.plan[index].changes.push(change);
        }
    }
    if planner.plan.is_empty() {
        return Ok(Plan {
            layers: Vec::new(),
            blocks: Vec::new(),
            width: planner.width,
            depth: 0.0,
            height: 0.0,
            line_width: line,
            extra_spacing: 1.0,
            gaps: BTreeMap::new(),
            brim_width: settings.brim_width.max(0.0),
            ribs: None,
            first_layer: 0,
            start_tool,
            y_shifts: Vec::new(),
            skip_points: Vec::new(),
            rib_offset: (0.0, 0.0),
            rib_footprint: (0.0, 0.0),
            skip_sparse: settings.skip_sparse,
            origin: (settings.x, settings.y),
        });
    }
    // `generate`: the filament in hand is the first change's old one, and
    // the tower is as tall as its last layer.
    if let Some(first) = planner.plan.iter().find_map(|layer| layer.changes.first()) {
        planner.current_tool = first.old;
    }
    planner.height = planner.plan.last().map_or(0.0, |layer| layer.z);

    // `plan_tower_new`.
    let brim_width = if settings.brim_width < 0.0 {
        auto_brim(planner.height)
    } else {
        settings.brim_width
    };
    planner.calc_block_infill_gap();
    let mut ribs = match settings.wall {
        Wall::Ribs {
            width,
            extra,
            fillet,
        } => Some(Ribs {
            width,
            length: 0.0,
            extra,
            fillet,
        }),
        Wall::Box => None,
    };
    if ribs.is_some() {
        // A ribbed tower is square, with the purge area it would have had
        // at the width set.
        planner.generate_blocks(false);
        let most = line + planner.blocks.iter().map(|b| b.depth).sum::<f32>();
        let square = align_ceil((most * planner.width * planner.extra_spacing).sqrt(), line);
        if !planner.blocks.is_empty() && square - 2.0 * line > EPSILON {
            planner.width = square;
            planner.replan_all();
        }
    }
    planner.generate_blocks(true);
    let mut most: f32 = planner.blocks.iter().map(|b| b.depth).sum();
    let least = limit_depth(planner.height);
    if settings.smooth_timelapse && most < EPSILON {
        most = least;
        // An idle ribbed tower is a square of the least depth.
        if ribs.is_some() {
            planner.width = most;
        }
    }
    if most > EPSILON && most + EPSILON < least {
        // Ribs reach the stability minimum by growing longer, a box by
        // spreading its rows.
        match &mut ribs {
            Some(_) => planner.rib_length = planner.rib_length.max(least * 2f32.sqrt()),
            None => planner.extra_spacing = (least / most).max(planner.extra_spacing),
        }
    }
    let first_layer = planner.first_layer.unwrap_or(0);
    let spacing = planner.extra_spacing;
    let across = planner.width - 2.0 * line;
    for (index, layer) in planner.plan.iter_mut().enumerate() {
        if index == first_layer {
            // The first layer is laid solid: its rows fill the depth kept
            // for them.
            layer.extra_spacing = 1.0;
            for change in &mut layer.changes {
                let wipe_depth = change.required_depth * spacing;
                change.required_depth = wipe_depth;
                change.wipe_length = wipe_depth / line * across;
                change.wipe_volume = length_to_volume(change.wipe_length, line, layer.height);
            }
        } else {
            layer.extra_spacing = spacing;
            for change in &mut layer.changes {
                change.required_depth *= spacing;
                change.wipe_length = volume_to_length(change.wipe_volume, line, layer.height);
            }
        }
    }
    planner.update_all_layer_depth(most);
    let skip_points = planner.wall_skip_points();
    if let Some(ribs) = &mut ribs {
        let (w, d) = (planner.width, planner.depth);
        let diagonal = (d * d + w * w).sqrt();
        planner.rib_length = planner.rib_length.max(diagonal) + ribs.extra;
        planner.rib_length = diagonal.max(planner.rib_length);
        ribs.length = planner.rib_length;
        ribs.width = ribs.width.min(d.min(w) / 2.0);
    }
    let y_shifts = planner.y_shifts();
    let mut plan = Plan {
        layers: planner.plan,
        blocks: planner.blocks,
        width: planner.width,
        depth: planner.depth,
        height: planner.height,
        line_width: line,
        extra_spacing: planner.extra_spacing,
        gaps: planner.gaps,
        brim_width,
        ribs,
        first_layer,
        start_tool: planner.current_tool,
        y_shifts,
        skip_points,
        rib_offset: (0.0, 0.0),
        rib_footprint: (0.0, 0.0),
        skip_sparse: settings.skip_sparse,
        origin: (settings.x, settings.y),
    };
    plan.set_rib_offset()?;
    Ok(plan)
}

/// A closed outline in mm as a region to clip.
fn region(points: &[(f64, f64)]) -> ExPolygon {
    crate::region(points)
}

impl Plan {
    /// Whether the tower is laid on a layer at all: it is deep enough, and
    /// with the layers with no tool change left out, it has one
    /// (`generate`, `plan_layer_is_laid`).
    pub fn laid(&self, layer: usize) -> bool {
        self.layers.get(layer).is_some_and(|planned| {
            planned.depth >= self.line_width && (!self.skip_sparse || !planned.changes.is_empty())
        })
    }

    /// The box the layer's outer wall goes round, in the tower's frame
    /// before the layer's y shift (`finish_layer_new`: a single block
    /// narrows with the layer, several keep the tower's depth;
    /// `align_perimeter`).
    pub fn wall_box(&self, layer: usize) -> Box2 {
        let depth = if self.blocks.len() == 1 {
            self.layers[layer].depth + self.line_width
        } else {
            self.depth
        };
        self.align_perimeter(Box2::new(0.0, 0.0, self.width, depth), self.y_shifts[layer])
    }

    /// `align_perimeter`: the wall's front and back on the row grid.
    pub(crate) fn align_perimeter(&self, area: Box2, y_shift: f32) -> Box2 {
        let pw = self.line_width;
        let spacing = self.extra_spacing * pw;
        let up = (align_ceil(area.top - pw, spacing) + pw).min(self.depth);
        let down = (align_floor(area.bottom - pw, spacing) + pw).max(-y_shift);
        Box2 {
            bottom: down,
            top: up,
            ..area
        }
    }

    /// The ribbed wall round `area` on a layer whose top is `z`, in the
    /// tower's frame before the layer's y shift (`generate_rib_polygon`,
    /// `rib_wall_polygon`).
    pub(crate) fn rib_wall(
        &self,
        ribs: &Ribs,
        area: &Box2,
        y_shift: f32,
        z: f32,
    ) -> Result<Vec<(f64, f64)>, ClipError> {
        let ring = rib_polygon(
            ribs,
            area,
            (self.width, self.depth, self.height),
            y_shift,
            z,
        )?;
        Ok(ring
            .iter()
            .map(|p| (unscaled(p.x), unscaled(p.y)))
            .collect())
    }

    /// `set_rib_offset`: the box round the first deep enough layer's wall,
    /// whose corner the tower is moved to stand at.
    fn set_rib_offset(&mut self) -> Result<(), ClipError> {
        self.rib_offset = (0.0, 0.0);
        self.rib_footprint = (0.0, 0.0);
        let Some(ribs) = self.ribs else {
            return Ok(());
        };
        let Some(first) = self
            .layers
            .iter()
            .position(|layer| layer.depth >= self.line_width)
        else {
            return Ok(());
        };
        let shift = self.y_shifts[first];
        let area = self.wall_box(first);
        let wall = self.rib_wall(&ribs, &area, shift, self.layers[first].z)?;
        let [left, bottom, right, top] = bounds(wall.iter().map(|p| (p.0, p.1 + f64::from(shift))));
        self.rib_offset = (-left as f32, -bottom as f32);
        self.rib_footprint = ((right - left) as f32, (top - bottom) as f32);
        Ok(())
    }

    /// How many brim loops the layer lays round its wall
    /// (`brim_chamfer_loops_num`): the whole brim on the tower's first
    /// layer, then one fewer each layer laid, at most 3 mm, and none once
    /// the layer is less deep than the first.
    pub fn brim_loops(&self, layer: usize) -> i32 {
        if layer >= self.layers.len() {
            return 0;
        }
        let spacing = self.line_width - self.layers[layer].height * (1.0 - PI / 4.0);
        let loops = ((self.brim_width + spacing / 2.0) / spacing) as i32;
        if layer == self.first_layer {
            return loops;
        }
        if self.layers[layer].depth != self.layers[0].depth {
            return 0;
        }
        let chamfer = (3.0 / spacing) as i32;
        let mut distance = layer as i32 - self.first_layer as i32;
        let mut at = self.first_layer + 1;
        while distance > 0 && at <= layer {
            if self.skip_sparse && self.layers[at].changes.is_empty() {
                distance -= 1;
            }
            at += 1;
        }
        if distance < 0 {
            0
        } else {
            loops.min(chamfer) - distance
        }
    }

    /// The outermost line the layer lays, wall or brim, as the box round
    /// its middle on the bed (what a G-code reader sees of the layer).
    pub fn layer_extent(&self, layer: usize) -> Result<[f64; 4], ClipError> {
        let shift = self.y_shifts[layer];
        let area = self.wall_box(layer);
        let spacing = self.line_width - self.layers[layer].height * (1.0 - PI / 4.0);
        let loops = self.brim_loops(layer).max(0);
        let (dx, dy) = (
            f64::from(self.origin.0 + self.rib_offset.0),
            f64::from(self.origin.1 + self.rib_offset.1 + shift),
        );
        let local = match self.ribs {
            Some(ribs) => {
                let mut ring = self.rib_wall(&ribs, &area, shift, self.layers[layer].z)?;
                for _ in 0..loops {
                    let grown = edge_clip::offset(
                        &[region(&ring)],
                        scaled(f64::from(spacing)),
                        Join::MITER,
                    )?;
                    let Some(next) = grown.first() else { break };
                    ring = next
                        .contour
                        .points
                        .iter()
                        .map(|p| (unscaled(p.x), unscaled(p.y)))
                        .collect();
                }
                bounds(ring.iter().copied())
            }
            None => {
                let outer = area.expand(loops as f32 * spacing);
                [
                    f64::from(outer.left),
                    f64::from(outer.bottom),
                    f64::from(outer.right),
                    f64::from(outer.top),
                ]
            }
        };
        Ok([local[0] + dx, local[1] + dy, local[2] + dx, local[3] + dy])
    }

    /// The footprint the reference reports (`get_footprint_size`): the
    /// box round the first layer's ribbed wall, or the tower's width and
    /// depth.
    pub fn footprint(&self) -> (f32, f32) {
        if self.ribs.is_some() {
            self.rib_footprint
        } else {
            (self.width, self.depth)
        }
    }

    /// The layers laid, and the top of the last (with the layers with no
    /// tool change left out, as tall as the layers laid).
    pub fn laid_layers(&self) -> (usize, f32) {
        let mut count = 0;
        let mut top = 0.0f32;
        let mut stacked = 0.0f32;
        for (index, layer) in self.layers.iter().enumerate() {
            let laid = self.laid(index);
            if !self.skip_sparse || laid {
                stacked += layer.height;
            }
            if laid {
                count += 1;
                top = if self.skip_sparse { stacked } else { layer.z };
            }
        }
        (count, top)
    }
}

/// `coord_t(v / SCALING_FACTOR)` in single precision (`scaled<coord_t>`
/// of a float).
fn scaled_f32(v: f32) -> i64 {
    (v / 1e-6f32) as i64
}

/// `coord_t(scale_(v))`: in double precision, cut towards zero
/// (`Point::new_scale`).
fn new_scale(v: f32) -> i64 {
    (f64::from(v) / 1e-6) as i64
}

fn pt(x: i64, y: i64) -> edge_geom::Point2 {
    edge_geom::Point2 { x, y }
}

fn area_of(points: Vec<edge_geom::Point2>) -> ExPolygon {
    ExPolygon {
        contour: edge_geom::Polygon { points },
        holes: Vec::new(),
    }
}

/// The ribbed wall of a layer whose top is `z`, round `area` (WipeTower.cpp
/// `generate_rib_polygon` and `rib_wall_polygon`), worked in the reference's
/// coordinate units and with its roundings, so that the outline has the
/// reference's corners.
fn rib_polygon(
    ribs: &Ribs,
    area: &Box2,
    (width, depth, height): (f32, f32, f32),
    y_shift: f32,
    z: f32,
) -> Result<Vec<edge_geom::Point2>, ClipError> {
    let half = scaled_f32(ribs.width) / 2;
    let lines = [
        (
            pt(new_scale(0.0), new_scale(0.0)),
            pt(new_scale(width), new_scale(depth)),
        ),
        (
            pt(new_scale(width), new_scale(0.0)),
            pt(new_scale(0.0), new_scale(depth)),
        ),
    ];
    let length = {
        let (a, b) = lines[0];
        ((b.x - a.x) as f64).hypot((b.y - a.y) as f64)
    };
    let over = (ribs.length - (length * 1e-6) as f32).max(0.0) / 2.0;
    let over = (height - z).abs() / height * over;
    let over = scaled_f32(over) as f32;
    let shift = scaled_f32(y_shift);
    let mut shapes = Vec::with_capacity(3);
    for (mut a, mut b) in lines {
        // `Line::extend`, then `translate(-y_shift)`.
        let (vx, vy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
        let norm = vx.hypot(vy);
        let ov = (
            (f64::from(over) * vx / norm) as i64,
            (f64::from(over) * vy / norm) as i64,
        );
        a = pt(a.x - ov.0, a.y - ov.1 - shift);
        b = pt(b.x + ov.0, b.y + ov.1 - shift);
        // `generate_rectange`.
        let (dx, dy) = ((b.x - a.x) as f64, (b.y - a.y) as f64);
        let l = (dx * dx + dy * dy).sqrt();
        let (ux, uy) = (dx / l, dy / l);
        let (ox, oy) = (-uy * half as f64, ux * half as f64);
        let round = |x: f64| x.round() as i64;
        shapes.push(area_of(vec![
            pt(round(a.x as f64 + ox), round(a.y as f64 + oy)),
            pt(round(a.x as f64 - ox), round(a.y as f64 - oy)),
            pt(round(b.x as f64 - ox), round(b.y as f64 - oy)),
            pt(round(b.x as f64 + ox), round(b.y as f64 + oy)),
        ]));
    }
    shapes.push(area_of(vec![
        pt(new_scale(area.left), new_scale(area.bottom)),
        pt(new_scale(area.right), new_scale(area.bottom)),
        pt(new_scale(area.right), new_scale(area.top)),
        pt(new_scale(area.left), new_scale(area.top)),
    ]));
    let first = |regions: Vec<ExPolygon>| {
        regions
            .into_iter()
            .next()
            .map(|r| r.contour.points)
            .unwrap_or_default()
    };
    let ribbed = first(edge_clip::union_ex(&shapes)?);
    if !ribs.fillet {
        return Ok(ribbed);
    }
    // `generate_rectange_polygon`: the box again, scaled in single
    // precision.
    let plain = area_of(vec![
        pt(scaled_f32(area.left), scaled_f32(area.bottom)),
        pt(scaled_f32(area.right), scaled_f32(area.bottom)),
        pt(scaled_f32(area.right), scaled_f32(area.top)),
        pt(scaled_f32(area.left), scaled_f32(area.top)),
    ]);
    Ok(first(edge_clip::union_ex(&[
        area_of(rounding_polygon(&ribbed)),
        plain,
    ])?))
}

/// WipeTower.cpp `rounding_polygon`: every corner turning between 30 and
/// 150 degrees replaced by an arc of 20 points tangent to both sides 2 mm
/// from it, or less on a short side.
fn rounding_polygon(points: &[edge_geom::Point2]) -> Vec<edge_geom::Point2> {
    const ROUNDING: f64 = 2.0;
    const PIECES: i32 = 20;
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let tolerance = (30.0f64 / 180.0 * std::f64::consts::PI).cos().abs();
    let mm = |p: edge_geom::Point2| (p.x as f64 * 1e-6, p.y as f64 * 1e-6);
    let to_units = |v: (f64, f64)| pt((v.0 / 1e-6) as i64, (v.1 / 1e-6) as i64);
    let mut out: Vec<edge_geom::Point2> = Vec::with_capacity(n * 2);
    for i in 0..n {
        let (a, b, c) = (
            mm(points[(i + n - 1) % n]),
            mm(points[i]),
            mm(points[(i + 1) % n]),
        );
        let ab_len = (a.0 - b.0).hypot(a.1 - b.1);
        let bc_len = (b.0 - c.0).hypot(b.1 - c.1);
        let ab = ((b.0 - a.0) / ab_len, (b.1 - a.1) / ab_len);
        let bc = ((c.0 - b.0) / bc_len, (c.1 - b.1) / bc_len);
        let cos = ((ab.0 * bc.0 + ab.1 * bc.1) as f32).clamp(-1.0, 1.0);
        let ccw = ab.0 * bc.1 - ab.1 * bc.0 > 0.0;
        if f64::from(cos.abs()) < tolerance {
            let reach = ROUNDING.min(ab_len / 2.1).min(bc_len / 2.1) as f32;
            let reach_d = f64::from(reach);
            let left = (b.0 - ab.0 * reach_d, b.1 - ab.1 * reach_d);
            let right = (b.0 + bc.0 * reach_d, b.1 + bc.1 * reach_d);
            let half_angle = cos.acos() / 2.0;
            let (dx, dy) = (right.0 - left.0, right.1 - left.1);
            let l = dx.hypot(dy);
            let mut dir = (-dy / l, dx / l);
            if !ccw {
                dir = (-dir.0, -dir.1);
            }
            let away = f64::from(reach) / f64::from(half_angle.sin());
            let center = (b.0 + dir.0 * away, b.1 + dir.1 * away);
            let radius = (left.0 - center.0).hypot(left.1 - center.1);
            // `ArcSegment` on the scaled centre, radius and ends.
            let c_u = to_units(center);
            let r_u = (radius / 1e-6) as i64 as f64;
            let (l_u, r_end) = (to_units(left), to_units(right));
            let polar = |p: edge_geom::Point2| {
                let t = ((p.y - c_u.y) as f64).atan2((p.x - c_u.x) as f64);
                if t < 0.0 {
                    2.0 * std::f64::consts::PI + t
                } else {
                    t
                }
            };
            let (start, end) = (polar(l_u), polar(r_end));
            let mut angle = end - start;
            if angle < 0.0 && ccw {
                angle += 2.0 * std::f64::consts::PI;
            } else if angle > 0.0 && !ccw {
                angle -= 2.0 * std::f64::consts::PI;
            }
            for j in 0..PIECES {
                let mut cur = (start + f64::from(j as f32 / PIECES as f32) * angle) as f32;
                if cur > 2.0 * std::f32::consts::PI {
                    cur -= 2.0 * std::f32::consts::PI;
                } else if cur < 0.0 {
                    cur += 2.0 * std::f32::consts::PI;
                }
                out.push(pt(
                    c_u.x + (r_u * f64::from(cur.cos())).round() as i64,
                    c_u.y + (r_u * f64::from(cur.sin())).round() as i64,
                ));
            }
            out.push(r_end);
        } else {
            out.push(points[i]);
        }
    }
    out.dedup();
    if out.len() > 1 && out.first() == out.last() {
        out.pop();
    }
    out
}

fn bounds(points: impl Iterator<Item = (f64, f64)>) -> [f64; 4] {
    points.fold(
        [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
        |[a, b, c, d], (x, y)| [a.min(x), b.min(y), c.max(x), d.max(y)],
    )
}

#[cfg(test)]
mod tests {
    use edge_config::Settings as Config;

    use super::*;

    fn settings(json: &str) -> Settings {
        Settings::from_settings(&Config::from_json(json).unwrap(), 0).unwrap()
    }

    /// `count` layers 0.2 mm thick, each changing between the filaments
    /// in `order`, in turn, as the tool orders of a two-colour print do.
    fn layers(count: usize, order: &[usize]) -> Vec<LayerIn> {
        let mut tool = order[0];
        (0..count)
            .map(|index| {
                let mut changes = Vec::new();
                for &next in order
                    .iter()
                    .cycle()
                    .skip(index % order.len())
                    .take(order.len())
                {
                    if next != tool {
                        changes.push(ChangeIn {
                            old: tool,
                            new: next,
                            purge_volume: 100.0,
                        });
                        tool = next;
                    }
                }
                LayerIn {
                    z: 0.2 * (index + 1) as f32,
                    height: 0.2,
                    changes,
                }
            })
            .collect()
    }

    const TWO_PLA: &str = r#"{"printer_model": "Bambu Lab A1", "prime_tower_width": "35", "filament_type": ["PLA", "PLA"],
        "filament_diameter": ["1.75", "1.75"]}"#;

    #[test]
    fn the_stability_table_and_the_auto_brim() {
        for (height, depth) in [
            (1.0, 5.0),
            (5.0, 5.0),
            (52.5, 12.5),
            (100.0, 20.0),
            (175.0, 30.0),
            (300.0, 50.0),
            (350.0, 60.0),
            (400.0, 60.0),
        ] {
            assert!((limit_depth(height) - depth).abs() < 1e-4, "{height}");
        }
        assert_eq!(auto_brim(50.0), 4.0);
        assert_eq!(auto_brim(150.0), 8.0);
    }

    #[test]
    fn two_filaments_share_one_block_of_whole_rows() {
        let s = settings(TWO_PLA);
        let plan = super::plan(&s, &layers(20, &[0, 1]), 0).unwrap();
        // 45 mm³ at 0.2 mm is 492 mm of row, 15 rows across 34 mm, at 150% of
        // a 0.5 mm line apart.
        assert_eq!(plan.blocks.len(), 1);
        assert!((plan.blocks[0].depth - 11.25).abs() < 1e-4);
        assert!((plan.depth - 11.75).abs() < 1e-4, "{}", plan.depth);
        assert_eq!(plan.extra_spacing, 1.0);
        assert_eq!(plan.gaps[&100], (0.75, 0.75));
        // The first layer is solid: its rows fill the depth, so it lays
        // 22.5 rows' worth.
        let first = &plan.layers[0].changes[0];
        assert!((first.required_depth - 11.25).abs() < 1e-4);
        assert!(
            (first.wipe_length - 22.5 * 34.0).abs() < 1e-2,
            "{}",
            first.wipe_length
        );
        let second = &plan.layers[1].changes[0];
        assert!((second.wipe_volume - 45.0).abs() < 1e-4);
        // Purges start on the wall, round the tower a layer at a time.
        let starts: Vec<(f32, f32)> = (0..4).map(|layer| plan.skip_points[layer][0]).collect();
        assert_eq!(starts[0], (0.0, 0.5));
        assert_eq!(starts[2], (35.0, 0.5));
        assert!((starts[1].1 - (0.5 + 11.25 - 0.75)).abs() < 1e-4);
        assert_eq!(starts[3].0, 0.0);
        // The brim on the first layer, a loop fewer each layer up to 3 mm.
        assert_eq!(plan.brim_loops(0), 7);
        assert_eq!(plan.brim_loops(1), 5);
        assert_eq!(plan.brim_loops(6), 0);
        assert_eq!(plan.laid_layers(), (20, 4.0));
        // The box round the first layer: 7 loops of 0.457 mm round the wall.
        let [left, bottom, right, top] = plan.layer_extent(0).unwrap();
        assert!((left - 11.8).abs() < 1e-3 && (bottom - 216.8).abs() < 1e-3);
        assert!((right - 53.2).abs() < 1e-3 && (top - 234.95).abs() < 1e-3);
        // As the estimate before slicing has it.
        let purge = PurgeEstimate {
            prime_volume: 45.0,
            category: 100,
            ..PurgeEstimate::default()
        };
        let estimate = estimate_tower_blocks_depth(&[purge, purge], 35.0, 0.2, 0.4, 1.5, 0.0, 4.0);
        assert!((estimate - plan.depth).abs() < 1e-4, "{estimate}");
    }

    #[test]
    fn a_ribbed_tower_is_squared_and_moved_to_its_wall() {
        let s = settings(
            r#"{"prime_tower_width": "35", "filament_type": ["PLA", "PLA"],
                "filament_diameter": ["1.75", "1.75"], "wipe_tower_wall_type": "rib"}"#,
        );
        let plan = super::plan(&s, &layers(20, &[0, 1]), 0).unwrap();
        // sqrt(11.75 x 35) up to a line: 20.5 wide, then 26 rows deep.
        assert_eq!(plan.width, 20.5);
        assert!((plan.depth - 20.0).abs() < 1e-4);
        let ribs = plan.ribs.unwrap();
        assert!((ribs.length - 20.5f32.hypot(20.0)).abs() < 1e-3);
        assert_eq!(ribs.width, 8.0);
        // Measured on the reference's G-code (a1_two_rib): the rows start
        // at 17.524, 222.554 with the tower set at 15, 220.
        assert!(
            (plan.rib_offset.0 - 2.024).abs() < 1e-3,
            "{:?}",
            plan.rib_offset
        );
        assert!(
            (plan.rib_offset.1 - 2.054).abs() < 1e-3,
            "{:?}",
            plan.rib_offset
        );
    }

    #[test]
    fn each_category_gets_a_block_and_a_short_tower_spreads_its_rows() {
        // PLA and PETG take blocks of their own, in the order they come.
        let s = settings(
            r#"{"prime_tower_width": "35", "filament_type": ["PLA", "PETG"],
                "filament_diameter": ["1.75", "1.75"]}"#,
        );
        let plan = super::plan(&s, &layers(20, &[0, 1]), 0).unwrap();
        let blocks: Vec<(i32, f32, f32)> = plan
            .blocks
            .iter()
            .map(|b| (b.category, b.start_depth, b.depth))
            .collect();
        assert_eq!(blocks, [(300, 0.5, 11.25), (100, 11.75, 11.25)]);
        assert!((plan.depth - 23.0).abs() < 1e-4);
        // Each layer changes from one to the other, so both are purged
        // throughout: no contact layers.
        assert!(
            plan.blocks
                .iter()
                .all(|b| b.layer_types.iter().all(|t| *t == LayerType::Normal))
        );

        // 5 mm³ a change on a 4 mm tower: under the 5 mm the table asks
        // for, so the rows are spread to it.
        let small = settings(
            r#"{"printer_model": "Bambu Lab A1", "prime_tower_width": "35",
                "filament_type": ["PLA", "PLA"], "filament_diameter": ["1.75", "1.75"],
                "filament_prime_volume": ["5", "5"]}"#,
        );
        let plan = super::plan(&small, &layers(20, &[0, 1]), 0).unwrap();
        // (5 mm³ is 55 mm of row: two rows, 1.5 mm.)
        assert!(
            (plan.extra_spacing - 5.0 / 1.5).abs() < 1e-3,
            "{}",
            plan.extra_spacing
        );
        assert!((plan.depth - 5.5).abs() < 1e-3);
        assert_eq!(plan.layers[0].extra_spacing, 1.0);
        assert!((plan.layers[1].extra_spacing - plan.extra_spacing).abs() < 1e-6);
    }

    #[test]
    fn a_category_that_stops_is_laid_solid_around_it_and_a_block_narrows() {
        let s = settings(TWO_PLA);
        // Changes on the first 10 layers only, then the tower stands on
        // without them up to layer 20: the single block keeps its depth
        // (a block is as deep as on any layer above).
        let mut input = layers(10, &[0, 1]);
        input.extend((10..20).map(|index| LayerIn {
            z: 0.2 * (index + 1) as f32,
            height: 0.2,
            changes: if index == 19 {
                vec![ChangeIn {
                    old: 1,
                    new: 0,
                    purge_volume: 0.0,
                }]
            } else {
                Vec::new()
            },
        }));
        let plan = super::plan(&s, &input, 0).unwrap();
        assert!(plan.layers.iter().all(|l| (l.depth - 11.25).abs() < 1e-4));
        assert!(plan.y_shifts.iter().all(|shift| *shift == 0.0));

        // Two changes a layer below, one above: the block narrows and the
        // layers above sit in the middle of the tower.
        let change = |old, new| ChangeIn {
            old,
            new,
            purge_volume: 0.0,
        };
        let input: Vec<LayerIn> = (0..20)
            .map(|index| LayerIn {
                z: 0.2 * (index + 1) as f32,
                height: 0.2,
                changes: if index < 10 {
                    vec![change(0, 1), change(1, 0)]
                } else {
                    vec![change(0, 1), change(1, 0)][..1].to_vec()
                },
            })
            .collect();
        let plan = super::plan(&s, &input, 0).unwrap();
        let depths: Vec<f32> = plan.layers.iter().map(|l| l.depth).collect();
        assert!((plan.layers[0].depth - 22.5).abs() < 1e-4, "{depths:?}");
        assert!((plan.layers[15].depth - 11.25).abs() < 1e-4);
        assert_eq!(plan.y_shifts[0], 0.0);
        // (23 - 11.25) / 2, to the 0.5 mm row spacing.
        assert_eq!(plan.y_shifts[15], 6.0);
        // No brim above once the layer is less deep than the first.
        assert_eq!(plan.brim_loops(12), 0);
    }

    #[test]
    fn what_is_not_planned_yet_is_refused() {
        let two = settings(r#"{"nozzle_diameter": ["0.4", "0.4"]}"#);
        assert!(matches!(
            super::plan(&two, &layers(2, &[0, 1]), 0),
            Err(PlanError::Unsupported(_))
        ));
        let combined = settings(r#"{"wipe_tower_sparse_layers_combination": "1"}"#);
        assert!(matches!(
            super::plan(&combined, &layers(2, &[0, 1]), 0),
            Err(PlanError::Unsupported(_))
        ));
        let narrow = settings(r#"{"prime_tower_width": "1"}"#);
        assert!(matches!(
            super::plan(&narrow, &layers(2, &[0, 1]), 0),
            Err(PlanError::TooNarrow { .. })
        ));
    }

    #[test]
    fn with_sparse_layers_left_out_the_tower_starts_at_its_first_change() {
        let s = settings(
            r#"{"prime_tower_width": "35", "filament_type": ["PLA", "PLA"],
                "filament_diameter": ["1.75", "1.75"], "wipe_tower_no_sparse_layers": "1"}"#,
        );
        let mut input = layers(6, &[0, 1]);
        for layer in [0, 2, 3] {
            input[layer].changes.clear();
        }
        let plan = super::plan(&s, &input, 0).unwrap();
        assert_eq!(plan.first_layer, 1);
        assert!(!plan.laid(0) && plan.laid(1) && !plan.laid(2) && plan.laid(4));
        // Layers 1, 4 and 5 are laid, the tower 0.6 mm tall.
        let (count, top) = plan.laid_layers();
        assert_eq!(count, 3);
        assert!((top - 0.6).abs() < 1e-6);
        // The brim steps in a loop per layer laid.
        assert_eq!(plan.brim_loops(1), 7);
        assert_eq!(plan.brim_loops(4), 5);
    }

    #[test]
    fn footprint_estimates() {
        let pla = PurgeEstimate {
            prime_volume: 45.0,
            category: 100,
            ..PurgeEstimate::default()
        };
        // Four filaments in one category: three purges a layer at worst.
        let four = estimate_tower_blocks_depth(&[pla; 4], 35.0, 0.2, 0.4, 1.5, 0.0, 6.2);
        assert!((four - 34.25).abs() < 1e-4, "{four}");
        // With a support filament of its own block, every purge counts.
        let support = PurgeEstimate {
            category: 702,
            ..pla
        };
        let mixed =
            estimate_tower_blocks_depth(&[pla, pla, support, pla], 35.0, 0.2, 0.4, 1.5, 0.0, 6.2);
        assert!((mixed - 45.5).abs() < 1e-4, "{mixed}");
        // A ribbed tower of two: 20.5 wide, 20 deep, ribs 8 wide.
        let side =
            estimate_rib_tower_bbox_side(&[pla, pla], 35.0, 0.2, 0.4, 1.5, 8.0, 0.0, 4.0, 0.0);
        assert!(side > 20.5 && side < 30.0, "{side}");
        assert!((estimate_brim_real_width(3.0, 0.4, 0.2, 0.0) - 3.428).abs() < 1e-3);
    }
}
