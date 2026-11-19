//! How the Type 1 tower is laid, layer by layer (WipeTower.cpp `generate`,
//! one nozzle): the outer wall and brim (`finish_layer_new`,
//! `print_outer_wall`), each tool change's purge (`tool_change_new`,
//! `get_next_pos`, `toolchange_wipe_new`), the rest of each block
//! (`finish_block`, `finish_block_solid`, `grid_fill`), the thick bridge
//! flow over a sparse layer (`get_floating_area`,
//! `need_thick_bridge_flow`), and how the parts of one visit are joined
//! (`merge_tcr`). Written as the reference's writer writes it
//! ([`super::writer`]), in the tower's frame.

use std::f32::consts::PI;

use edge_clip::Join;
use edge_geom::{ExPolygon, Polygon};

use super::gaps::{GapPoint, cut_wall_gaps};
use super::plan::{Box2, LayerType, Plan, PlanError};
use super::settings::Settings;
use super::writer::{V2, Writer, fixed, scaled};

const EPSILON: f32 = 1e-4;
const WT_EPSILON: f32 = 1e-3;
/// The furthest apart the lines across unused depth are (`m_bridging`).
const BRIDGING: f32 = 10.0;
/// How fast a contact change and a contact block are laid, in mm/min
/// (`m_contact_speed`).
const CONTACT_SPEED: f32 = 20.0 * 60.0;

/// What the tower lays in one visit (`ToolChangeResult`), in the tower's
/// frame with the rib offset applied.
#[derive(Clone, Debug, Default)]
pub struct Visit {
    pub gcode: String,
    pub start_pos: V2,
    pub end_pos: V2,
    pub initial_tool: usize,
    pub new_tool: usize,
    pub print_z: f32,
    pub layer_height: f32,
    pub is_tool_change: bool,
    /// Where the purge of a tool change starts (`tool_change_start_pos`).
    pub tool_change_start_pos: V2,
    pub wipe_path: Vec<V2>,
    /// What the change G-code flushes, in mm³.
    pub purge_volume: f32,
    /// The change G-code may cool the nozzle before the tower: the tower
    /// heats it back up (`reheats_after_cooling`).
    pub reheats_after_cooling: bool,
    pub is_contact: bool,
    pub is_finish_first: bool,
}

/// `is_valid_gcode`: a line that is neither empty nor a comment.
fn has_moves(gcode: &str) -> bool {
    gcode.split('\n').any(|line| {
        let line = line.trim_matches(' ');
        !line.is_empty() && !line.starts_with(';')
    })
}

#[derive(Clone, Debug)]
struct BlockState {
    cur_depth: f32,
    finish_depth: Vec<f32>,
    last_filament_change: Option<usize>,
}

struct Layer<'a> {
    plan: &'a Plan,
    settings: &'a Settings,
    blocks: Vec<BlockState>,
    current_tool: usize,
    left_to_right: bool,
    tool_changes: usize,
    layer_changes: usize,
    /// The layer being laid, its top, thickness and flow.
    index: usize,
    z: f32,
    height: f32,
    flow: f32,
    y_shift: f32,
    cur_block: Option<usize>,
    max_e_speed: Vec<f32>,
}

/// The rectangle through two corners (`generate_rectange_polygon`).
fn box_polygon(area: &Box2) -> Polygon {
    Polygon {
        points: vec![
            scaled((area.left, area.bottom)),
            scaled((area.right, area.bottom)),
            scaled((area.right, area.top)),
            scaled((area.left, area.top)),
        ],
    }
}

impl Layer<'_> {
    fn pw(&self) -> f32 {
        self.plan.line_width
    }

    fn width(&self) -> f32 {
        self.plan.width
    }

    fn depth(&self) -> f32 {
        self.plan.depth
    }

    /// `extrusion_flow`: the tower line's cross-section over filament 1's.
    fn flow_at(&self, height: f32) -> f32 {
        let area = self.settings.filaments.first().map_or(1.0, |f| f.area);
        height * (self.pw() - height * (1.0 - PI / 4.0)) / area
    }

    fn is_first_layer(&self) -> bool {
        self.index == self.plan.first_layer
    }

    fn category(&self, filament: usize) -> i32 {
        self.settings.category(filament)
    }

    fn block_of(&self, filament: usize) -> Option<usize> {
        let category = self.category(filament);
        self.plan.blocks.iter().position(|b| b.category == category)
    }

    /// `calc_tower_feedrate`, one nozzle.
    fn tower_feedrate(&self, tool: usize, first_layer: bool) -> f32 {
        let max = self.settings.max_speed;
        if first_layer {
            return (self.settings.first_layer_speed * 60.0).min(max);
        }
        let e = self.max_e_speed.get(tool).copied().unwrap_or(f32::MAX);
        (60.0 * e / self.flow).min(max)
    }

    fn writer(&self, height: f32, flow: f32, tool: usize) -> Writer {
        Writer::new(
            height,
            self.pw(),
            flow,
            tool,
            self.plan.rib_offset,
            self.settings.arc_fitting,
            self.y_shift,
            self.max_e_speed.clone(),
        )
    }

    fn laid(&self, layer: usize) -> bool {
        layer < self.plan.layers.len()
            && (!self.plan.skip_sparse || !self.plan.layers[layer].changes.is_empty())
    }

    /// `get_floating_area`: the part of the current block the layer below
    /// left sparse.
    fn floating_area(&self) -> Option<(f32, f32)> {
        if self.index <= 1 {
            return None;
        }
        let block = self.cur_block?;
        let mut below = self.index - 1;
        while below > 0 && !self.laid(below) {
            below -= 1;
        }
        if below <= self.plan.first_layer || !self.laid(below) {
            return None;
        }
        let planned = &self.plan.blocks[block];
        let end = planned.start_depth + planned.depth - self.pw();
        let start = self.blocks[block].finish_depth[below];
        Some((start, end))
    }

    /// `need_thick_bridge_flow`: a line at `y` over the open part of the
    /// layer below is laid as thick as a 0.2 mm layer's.
    fn needs_bridge_flow(&self, y: f32, height: f32) -> bool {
        if height >= 0.2 {
            return false;
        }
        self.floating_area()
            .is_some_and(|(start, end)| y > start && y < end)
    }

    /// `next_layer_purges_into`.
    fn next_layer_purges_into(&self, category: i32) -> bool {
        if self.plan.blocks.len() < 2 || self.index + 1 >= self.plan.layers.len() {
            return false;
        }
        self.plan.layers[self.index + 1]
            .changes
            .iter()
            .any(|change| self.category(change.new) == category)
    }

    /// `block_row_pitch`.
    fn block_row_pitch(&self, category: i32) -> f32 {
        let gap = self.plan.gaps.get(&category).map_or(self.pw(), |gap| gap.0);
        self.plan.layers[self.index].extra_spacing * gap
    }

    /// `grid_fill`: rows on the block's purge lattice, strictly inside.
    fn grid_fill(&self, writer: &mut Writer, fill: &Box2, origin: f32, row: f32, f: f32) {
        let row = if row < self.pw() - WT_EPSILON {
            self.pw()
        } else {
            row
        };
        let mut y = origin + ((fill.bottom - origin) / row - 1e-4).ceil() * row;
        if y < fill.bottom + WT_EPSILON {
            y += row;
        }
        let (xl, xr) = (fill.left, fill.right);
        let mut left_to_right = (writer.x() - xl).abs() <= (writer.x() - xr).abs();
        while y < fill.top - WT_EPSILON {
            writer.travel((if left_to_right { xl } else { xr }, y), 0.0);
            writer.extrude((if left_to_right { xr } else { xl }, y), f);
            left_to_right = !left_to_right;
            y += row;
        }
    }

    /// Whether the next layer has a change to or from a soluble filament,
    /// or this is the first layer: the fill is solid then.
    fn solid_infill(&self, first_layer: bool) -> bool {
        let soluble = |f: usize| self.settings.filaments.get(f).is_some_and(|f| f.is_soluble);
        let next = self.plan.layers.get(self.index + 1).is_some_and(|next| {
            next.changes
                .iter()
                .any(|change| soluble(change.new) || soluble(change.old))
        });
        next || first_layer
    }

    /// The rectangle round a fill box (`rectangle_fill_box` with the wipe
    /// path), and its wipe.
    fn fill_rectangle(&self, writer: &mut Writer, fill: &Box2, f: f32, wipe: &mut Vec<V2>) {
        let width = fill.right - fill.left;
        let height = fill.top - fill.bottom;
        let pw = self.pw();
        if height > pw - 1e-3 {
            let bridge = self
                .needs_bridge_flow(fill.bottom, self.height)
                .then(|| self.flow_at(0.2));
            writer.rectangle_fill_box((fill.left, fill.bottom), width, height, f, bridge);
            let corners = [
                (fill.left, fill.bottom),
                (fill.right, fill.bottom),
                (fill.right, fill.top),
                (fill.left, fill.top),
            ];
            let at = writer.pos();
            let target = if at == corners[0] {
                corners[1]
            } else if at == corners[1] {
                corners[2]
            } else if at == corners[2] {
                corners[3]
            } else {
                corners[0]
            };
            wipe.push(at);
            wipe.push(target);
        } else if height > 1e-3 {
            let (ld, rd) = ((fill.left, fill.bottom), (fill.right, fill.bottom));
            let d = |p: V2| {
                let (dx, dy) = (writer.x() - p.0, writer.y() - p.1);
                dx * dx + dy * dy
            };
            let from_left = d(ld) <= d(rd);
            writer.travel(if from_left { ld } else { rd }, 0.0);
            writer.extrude(if from_left { rd } else { ld }, f);
            wipe.push(writer.pos());
            wipe.push(if from_left { ld } else { rd });
        }
    }

    /// The fill of a box: solid on the first layer or under a soluble
    /// filament, rows on the purge grid where the next layer purges into
    /// the block, else sparse lines `BRIDGING` apart with an inverse U.
    #[allow(clippy::too_many_arguments)]
    fn fill(
        &self,
        writer: &mut Writer,
        fill: &Box2,
        f: f32,
        first_layer: bool,
        grid: Option<(f32, f32)>,
        wipe: &mut Vec<V2>,
    ) {
        let pw = self.pw();
        let dy = fill.top - fill.bottom - pw;
        let mut left = fill.left + 2.0 * pw;
        let mut right = fill.right - 2.0 * pw;
        if dy <= pw {
            return;
        }
        writer.travel((fill.left + pw * 2.0, fill.bottom), 0.0);
        writer.append(";--------------------\n; CP EMPTY GRID START\n");
        writer.comment_with_value(" layer #", self.layer_changes + 1);
        let solid = self.solid_infill(first_layer);
        if let Some((origin, row)) = grid.filter(|_| !solid) {
            self.grid_fill(writer, fill, origin, row, f);
        } else if solid {
            let mut sparse_factor = 1.5f32;
            if first_layer {
                left -= pw;
                right += pw;
                sparse_factor = 1.0;
            }
            let mut y = fill.bottom + pw;
            let n = (dy / (pw * sparse_factor)) as i32;
            let spacing = (dy - pw) / (n - 1) as f32;
            for i in 0..n {
                let x = writer.x();
                writer.extrude((x, y), f);
                writer.extrude((if i % 2 == 1 { left } else { right }, y), 0.0);
                y += spacing;
            }
            let x = writer.x();
            writer.extrude((x, fill.top), 0.0);
        } else {
            writer.extrude((fill.left + pw * 2.0, fill.top), f);
            let n = 1 + ((right - left) / BRIDGING) as i32;
            let dx = (right - left) / n as f32;
            for i in 1..=n {
                let x = left + dx * i as f32;
                let y = writer.y();
                writer.travel((x, y), 0.0);
                writer.extrude((x, if i % 2 == 1 { fill.bottom } else { fill.top }), 0.0);
            }
            wipe.clear();
            wipe.push(writer.pos());
            wipe.push((
                left + dx * n as f32,
                if n % 2 == 1 { fill.top } else { fill.bottom },
            ));
        }
        writer.append("; CP EMPTY GRID END\n;------------------\n\n\n\n\n\n\n");
    }

    /// The layer's outer wall polygon round `area` (`print_outer_wall`
    /// without laying it).
    fn wall_polygon(&self, area: &Box2) -> Result<Polygon, PlanError> {
        Ok(match &self.plan.ribs {
            Some(ribs) => {
                let ring = self.plan.rib_wall(ribs, area, self.y_shift, self.z)?;
                Polygon {
                    points: ring
                        .iter()
                        .map(|(x, y)| edge_geom::Point2::from_mm(*x, *y))
                        .collect(),
                }
            }
            None => box_polygon(area),
        })
    }

    /// `print_outer_wall`: the wall, left open where purges come in.
    fn print_outer_wall(
        &self,
        writer: &mut Writer,
        area: &Box2,
        f: f32,
        extrude: bool,
    ) -> Result<Polygon, PlanError> {
        let wall = self.wall_polygon(area)?;
        if !extrude {
            return Ok(wall);
        }
        let gaps: Vec<GapPoint> = self.plan.skip_points[self.index]
            .iter()
            .map(|p| GapPoint {
                pos: *p,
                is_left: p.0 < self.width() / 2.0,
            })
            .collect();
        let ribbed = self.plan.ribs.is_some();
        if gaps.is_empty() {
            if ribbed {
                writer.polygon(&wall, f, false);
            } else {
                writer.rectangle(
                    (area.left, area.bottom),
                    area.right - area.left,
                    area.top - area.bottom,
                    f,
                );
            }
        } else {
            let own =
                &self.settings.filaments[self.current_tool.min(self.settings.filaments.len() - 1)];
            let pieces = cut_wall_gaps(&wall, &gaps, 2.5 * self.pw());
            writer.gap_wall(
                &pieces,
                f,
                own.retract_length,
                own.retract_speed * 60.0,
                ribbed,
            );
        }
        Ok(wall)
    }

    /// `finish_layer_new`, one nozzle: the outer wall and brim, and on a
    /// layer with no change the fill of every block.
    fn finish_layer(
        &mut self,
        extrude_perimeter: bool,
        extrude_fill: bool,
        extrude_fill_wall: bool,
    ) -> Result<Visit, PlanError> {
        let old_tool = self.current_tool;
        let pw = self.pw();
        let mut writer = self.writer(self.height, self.flow, self.current_tool);
        writer.append("; WIPE_TOWER_START\n");
        let first_layer = self.is_first_layer();
        let feedrate = self.tower_feedrate(self.current_tool, first_layer);
        let planned = &self.plan.layers[self.index];
        let mut wipe = Vec::new();
        let multi_block = self.plan.blocks.len() > 1 && (extrude_fill_wall || extrude_fill);
        let mut fills: Vec<(Box2, Option<(i32, f32)>)> = Vec::new();
        if multi_block {
            for block in &self.plan.blocks {
                let height = block.layer_depths[self.index] - 2.0 * pw;
                if height <= WT_EPSILON {
                    continue;
                }
                fills.push((
                    Box2::new(pw, block.start_depth + pw, self.width() - 2.0 * pw, height),
                    Some((block.category, block.start_depth)),
                ));
            }
        }
        if fills.is_empty() {
            let depth = if self.plan.blocks.len() == 1 {
                planned.depth - 2.0 * pw
            } else {
                self.depth() - 2.0 * pw
            };
            fills.push((Box2::new(pw, pw, self.width() - 2.0 * pw, depth), None));
        }
        let first = fills[0].0;
        writer.set_initial_position(
            if self.left_to_right {
                (first.right, first.top)
            } else {
                (first.left, first.top)
            },
            self.width(),
            self.depth(),
        );
        let solid = self.solid_infill(first_layer);
        for (index, (fill, block)) in fills.iter().enumerate() {
            let grid = block
                .filter(|(category, _)| !solid && self.next_layer_purges_into(*category))
                .map(|(category, origin)| (origin, self.block_row_pitch(category)));
            if index > 0 {
                writer.travel(
                    if self.left_to_right {
                        (fill.right, fill.top)
                    } else {
                        (fill.left, fill.top)
                    },
                    0.0,
                );
            }
            if extrude_fill_wall && fill.top - fill.bottom > WT_EPSILON {
                self.fill_rectangle(&mut writer, fill, feedrate, &mut wipe);
            }
            if extrude_fill {
                self.fill(&mut writer, fill, feedrate, first_layer, grid, &mut wipe);
            }
        }
        // The wall: a single block narrows with the layer.
        let depth = if self.plan.blocks.len() == 1 {
            planned.depth + pw
        } else {
            self.depth()
        };
        let mut area = self
            .plan
            .align_perimeter(Box2::new(0.0, 0.0, self.width(), depth), self.y_shift);
        let mut outer = self.print_outer_wall(&mut writer, &area, feedrate, extrude_perimeter)?;
        let spacing = pw - self.height * (1.0 - PI / 4.0);
        let loops = self.plan.brim_loops(self.index);
        if loops > 0 && self.plan.ribs.is_some() {
            for _ in 0..loops {
                let grown = edge_clip::offset(
                    &[ExPolygon {
                        contour: without_short_edges(&outer, spacing),
                        holes: Vec::new(),
                    }],
                    edge_geom::scaled(f64::from(spacing)),
                    Join::MITER,
                )?;
                let Some(next) = grown.into_iter().next() else {
                    break;
                };
                outer = next.contour;
                writer.polygon(&outer, feedrate, true);
            }
        } else if loops > 0 {
            for _ in 0..loops {
                area = Box2 {
                    left: area.left - spacing,
                    bottom: area.bottom - spacing,
                    right: area.right + spacing,
                    top: area.top + spacing,
                };
                writer.rectangle(
                    (area.left, area.bottom),
                    area.right - area.left,
                    area.top - area.bottom,
                    feedrate,
                );
            }
        }
        if self.plan.ribs.is_some() && (extrude_perimeter || loops > 0) {
            let reach = self.settings.filaments
                [self.current_tool.min(self.settings.filaments.len() - 1)]
            .wipe_distance;
            writer.add_wipe_path(&outer, f64::from(reach));
        } else {
            let corners = [
                (area.left, area.bottom),
                (area.right, area.bottom),
                (area.right, area.top),
                (area.left, area.top),
            ];
            let at = writer.pos();
            let mut target = if at == corners[0] {
                corners[1]
            } else if at == corners[1] {
                corners[2]
            } else if at == corners[2] {
                corners[3]
            } else {
                corners[0]
            };
            if wipe.len() == 2 && wipe[0] == at {
                target = wipe[1];
            }
            writer.add_wipe_point(at).add_wipe_point(target);
        }
        writer.append("; WIPE_TOWER_END\n");
        Ok(self.visit(writer, old_tool, true, false, 0.0, false))
    }

    /// `only_generate_out_wall`: a smooth timelapse's wall, the tower
    /// laid whole on every layer, by the filament in hand.
    fn only_wall(&mut self) -> Result<Visit, PlanError> {
        let old_tool = self.current_tool;
        let pw = self.pw();
        let mut writer = self.writer(self.height, self.flow, self.current_tool);
        let feedrate = self.tower_feedrate(self.current_tool, self.is_first_layer());
        let planned = &self.plan.layers[self.index];
        let used: f32 = planned.changes.iter().map(|c| c.required_depth).sum();
        let fill_y = used + pw;
        let fill = Box2::new(pw, fill_y, self.width() - 2.0 * pw, planned.depth - fill_y);
        writer.set_initial_position(
            if self.left_to_right {
                (fill.right, fill.top)
            } else {
                (fill.left, fill.top)
            },
            self.width(),
            self.depth(),
        );
        writer.append("; WIPE_TOWER_START\n");
        let area = self.plan.align_perimeter(
            Box2::new(0.0, 0.0, self.width(), self.depth()),
            self.y_shift,
        );
        let wall = self.print_outer_wall(&mut writer, &area, feedrate, true)?;
        if self.plan.ribs.is_some() {
            let reach = self.settings.filaments
                [self.current_tool.min(self.settings.filaments.len() - 1)]
            .wipe_distance;
            writer.add_wipe_path(&wall, f64::from(reach));
        } else {
            let corners = [
                (area.left, area.bottom),
                (area.right, area.bottom),
                (area.right, area.top),
                (area.left, area.top),
            ];
            let at = writer.pos();
            let target = if at == corners[0] {
                corners[1]
            } else if at == corners[1] {
                corners[2]
            } else if at == corners[2] {
                corners[3]
            } else {
                corners[0]
            };
            writer.add_wipe_point(at).add_wipe_point(target);
        }
        writer.append("; WIPE_TOWER_END\n");
        Ok(self.visit(writer, old_tool, true, false, 0.0, false))
    }

    /// `construct_tcr`.
    fn visit(
        &self,
        writer: Writer,
        old_tool: usize,
        is_finish: bool,
        is_tool_change: bool,
        purge_volume: f32,
        is_contact: bool,
    ) -> Visit {
        let start_pos = writer.start_rotated();
        Visit {
            start_pos,
            end_pos: writer.pos_rotated(),
            initial_tool: old_tool,
            new_tool: self.current_tool,
            print_z: self.z,
            layer_height: self.height,
            is_tool_change,
            tool_change_start_pos: if is_tool_change {
                start_pos
            } else {
                (0.0, 0.0)
            },
            wipe_path: writer.wipe_path.clone(),
            purge_volume,
            reheats_after_cooling: false,
            is_contact,
            is_finish_first: is_finish,
            gcode: writer.gcode,
        }
    }

    /// `wipe_line_spacing`.
    fn row_spacing(&self, tool: usize, solid: bool) -> f32 {
        if solid || self.is_first_layer() {
            return self.pw();
        }
        let gap = self
            .plan
            .gaps
            .get(&self.category(tool))
            .map_or(self.pw(), |gap| gap.0);
        self.plan.layers[self.index].extra_spacing * gap
    }

    /// `get_next_pos`: the purge's first row, at a corner that goes round
    /// with the layers.
    fn next_pos(&self, cleaning: &Box2, wipe_length: f32, tool: usize, solid: bool) -> V2 {
        let (xl, xr) = (cleaning.left, cleaning.right);
        let dy = self.row_spacing(tool, solid);
        let depth = cleaning.top - cleaning.bottom;
        let mut y_offset = (depth - dy).max(0.0);
        if !solid && xr > xl {
            let lines = 1.max((wipe_length / (xr - xl)).ceil() as i32);
            y_offset = ((lines - 1) as f32 * dy).min(y_offset);
        }
        match self.index % 4 {
            0 => (cleaning.left, cleaning.bottom),
            1 => (cleaning.right, cleaning.bottom + y_offset),
            2 => (cleaning.right, cleaning.bottom),
            _ => (cleaning.left, cleaning.bottom + y_offset),
        }
    }

    /// `tool_change_new`, one nozzle.
    fn tool_change(&mut self, new_tool: usize, solid: bool) -> Result<Visit, PlanError> {
        let old_tool = self.current_tool;
        let planned = self.plan.layers[self.index]
            .changes
            .iter()
            .find(|change| change.new == new_tool)
            .copied();
        let wipe_length = planned.map_or(0.0, |c| c.wipe_length);
        let wipe_depth = planned.map_or(0.0, |c| c.required_depth);
        let purge_volume = planned.map_or(0.0, |c| c.purge_volume);
        let block = self.block_of(new_tool).ok_or_else(|| {
            PlanError::Unsupported(format!("no block planned for filament {}", new_tool + 1))
        })?;
        self.cur_block = Some(block);
        let pw = self.pw();
        let cleaning = Box2::new(
            pw,
            self.blocks[block].cur_depth,
            self.width() - 2.0 * pw,
            wipe_depth,
        );
        let first_layer = self.is_first_layer();
        let mut writer = self.writer(self.height, self.flow, self.current_tool);
        let material = |tool: usize| {
            self.settings
                .filaments
                .get(tool)
                .map_or_else(|| "(NONE)".to_owned(), |f| f.material.clone())
        };
        writer.append(";--------------------\n; CP TOOLCHANGE START\n");
        writer.comment_with_value(" toolchange #", self.tool_changes + 1);
        writer.append(&format!(
            "; material : {} -> {}\n;--------------------\n",
            material(self.current_tool),
            material(new_tool)
        ));
        writer.append("M220 S100\n");
        let initial = self.next_pos(&cleaning, wipe_length, new_tool, solid);
        writer.set_initial_position(initial, self.width(), self.depth());
        writer.append("; WIPE_TOWER_START\n");
        // `toolchange_Change`: the change itself is the printer's G-code.
        writer.append(
            "[filament_end_gcode]\n[restore_layer_z_before_toolchange]\n[change_filament_gcode]\n\
             [deretraction_from_wipe_tower_generator]\n",
        );
        writer.set_tool(new_tool);
        writer.append("[filament_start_gcode]\n");
        writer.append("G4 S0\n");
        self.current_tool = new_tool;
        let reheats = !first_layer && !solid;
        self.wipe(&mut writer, &cleaning, wipe_length, solid, reheats);
        writer.append("; WIPE_TOWER_END\n");
        self.tool_changes += 1;
        self.blocks[block].cur_depth += wipe_depth;
        self.blocks[block].last_filament_change = Some(new_tool);
        writer.feedrate(self.settings.travel_speed * 60.0);
        writer.append("G4 S0\nG92 E0\n; CP TOOLCHANGE END\n;------------------\n\n\n");
        let mut visit = self.visit(writer, old_tool, false, true, purge_volume, solid);
        visit.reheats_after_cooling = reheats;
        Ok(visit)
    }

    /// The built-in speed ramp of the purge rows, row `row` (from 0): the
    /// map holds its fifth step for every row after, as Bambu Studio's does.
    fn ramp(target: f32, row: usize) -> f32 {
        let steps = [
            0.33 * target,
            0.375 * target,
            0.458 * target,
            0.875 * target,
            target.min(0.875 * target + 50.0),
        ];
        steps[row.min(steps.len() - 1)]
    }

    /// `toolchange_wipe_new`: the purge, row by row.
    fn wipe(
        &mut self,
        writer: &mut Writer,
        cleaning: &Box2,
        wipe_length: f32,
        solid: bool,
        reheat: bool,
    ) {
        let tool = self.current_tool;
        let own = self.settings.filaments[tool.min(self.settings.filaments.len() - 1)].clone();
        let pw = self.pw();
        let height = self.height;
        let tool_flow = self.flow;
        let first_layer = self.is_first_layer();
        writer.set_flow(
            tool_flow
                * if first_layer {
                    self.settings.first_layer_flow_ratio
                } else {
                    1.0
                },
        );
        writer.append("; CP TOOLCHANGE WIPE\n");
        if first_layer {
            writer.change_line_width(self.settings.first_layer_flow_ratio * pw);
        }
        let retract_length = own.retract_length;
        let retract_speed = own.retract_speed * 60.0;
        let (xl, xr) = (cleaning.left, cleaning.right);
        let line_ironing = self.settings.line_ironing && !solid;
        let flat_ironing = self.settings.flat_ironing && line_ironing;
        let mut x_to_wipe = if solid { f32::MAX } else { wipe_length };
        let dy = self.row_spacing(tool, solid);
        let mut target = if first_layer {
            (self.settings.first_layer_speed * 60.0).min(self.settings.max_speed)
        } else {
            self.tower_feedrate(tool, false)
        };
        if solid {
            target = CONTACT_SPEED;
        }
        let curve: Option<Vec<f32>> = (!own.speed_factor_curve.is_empty()).then(|| {
            let mut map: Vec<f32> = own.speed_factor_curve.iter().map(|v| v * target).collect();
            while map.len() < 5 {
                map.push(*map.last().unwrap_or(&target));
            }
            map
        });
        let mut wipe_speed = 0.0f32;
        self.left_to_right = (self.index + 3) % 4 >= 2;
        let from_up = self.index % 2 == 1;
        for i in 0usize.. {
            wipe_speed = match &curve {
                Some(map) => map.get(i).copied().unwrap_or(wipe_speed),
                None => Self::ramp(target, i),
            };
            let bridge = self.needs_bridge_flow(writer.y(), height);
            if bridge {
                writer.set_flow(self.flow_at(0.2));
                writer.height_tag(0.2);
            }
            if i == 0 && reheat && own.cooling_before_tower > EPSILON {
                writer.append(&format!(
                    "M104 T0 S{} N0 W ;Wipe tower reheat before wipe\n",
                    own.temperature
                ));
            }
            if i == 0 && self.settings.wall_gap {
                let mut ironing = 3.0f32;
                let edge = if self.left_to_right { xr } else { xl };
                let sign = if self.left_to_right { 1.0 } else { -1.0 };
                if line_ironing {
                    let dx = edge - writer.x();
                    if dx.abs() < ironing {
                        ironing = dx.abs();
                    }
                    let (x, y) = (writer.x(), writer.y());
                    writer.extrude((x + sign * ironing, y), wipe_speed);
                    writer.retract(retract_length, retract_speed);
                    let (x, y) = (writer.x(), writer.y());
                    writer.travel((x - sign * 1.5 * ironing, y), 600.0);
                    if flat_ironing {
                        let (x, y) = (writer.x(), writer.y());
                        writer.travel((x + sign * 0.5 * ironing, y), 240.0);
                        let back = (writer.x() + sign * ironing, writer.y());
                        let at = writer.pos();
                        writer.spiral_flat_ironing(
                            at,
                            own.iron_area,
                            pw,
                            self.settings.ironing_speed,
                        );
                        writer.travel(back, wipe_speed);
                    } else {
                        let (x, y) = (writer.x(), writer.y());
                        writer.travel((x + sign * 1.5 * ironing, y), 240.0);
                    }
                    writer.retract(-retract_length, retract_speed);
                }
                let y = writer.y();
                writer.extrude((edge, y), wipe_speed);
            } else {
                let y = writer.y();
                writer.extrude((if self.left_to_right { xr } else { xl }, y), wipe_speed);
            }
            if bridge {
                writer.set_flow(tool_flow);
                writer.height_tag(height);
            }
            if !from_up && writer.y() + dy - EPSILON > cleaning.top - pw {
                break;
            }
            if from_up && writer.y() - dy + EPSILON < cleaning.bottom {
                break;
            }
            x_to_wipe -= xr - xl;
            if x_to_wipe < WT_EPSILON {
                break;
            }
            let (x, y) = (writer.x(), writer.y());
            writer.extrude((x, y + if from_up { -dy } else { dy }), 0.0);
            self.left_to_right = !self.left_to_right;
        }
        let (x, y) = (writer.x(), writer.y());
        writer.add_wipe_point((x, y)).add_wipe_point((
            if !self.left_to_right {
                self.width()
            } else {
                0.0
            },
            y,
        ));
        if self.plan.layers[self.index]
            .changes
            .last()
            .is_some_and(|last| last.new != self.current_tool)
        {
            self.left_to_right = !self.left_to_right;
        }
        writer.set_flow(self.flow);
        if first_layer {
            writer.change_line_width(pw);
        }
    }

    /// `finish_block`: the rest of a block no change filled, as sparse fill
    /// with its rectangle.
    fn finish_block(&mut self, block: usize, filament: usize) -> Result<Visit, PlanError> {
        let pw = self.pw();
        let planned = &self.plan.blocks[block];
        let mut writer = self.writer(self.height, self.flow, filament);
        writer.append("; WIPE_TOWER_START\n");
        let first_layer = self.is_first_layer();
        let feedrate = self.tower_feedrate(filament, first_layer);
        let cur = self.blocks[block].cur_depth;
        let fill = Box2::new(
            pw,
            cur,
            self.width() - 2.0 * pw,
            planned.start_depth + planned.layer_depths[self.index] - cur - pw,
        );
        writer.set_initial_position(
            if self.left_to_right {
                (fill.right, fill.top)
            } else {
                (fill.left, fill.top)
            },
            self.width(),
            self.depth(),
        );
        let mut wipe = Vec::new();
        if fill.top - fill.bottom > WT_EPSILON {
            self.fill_rectangle(&mut writer, &fill, feedrate, &mut wipe);
        }
        let grid = self
            .next_layer_purges_into(planned.category)
            .then(|| (fill.bottom, self.block_row_pitch(planned.category)));
        self.fill(&mut writer, &fill, feedrate, first_layer, grid, &mut wipe);
        let area = self.plan.align_perimeter(
            Box2::new(
                0.0,
                0.0,
                self.width(),
                self.plan.layers[self.index].depth + pw,
            ),
            self.y_shift,
        );
        let corners = [
            (area.left, area.bottom),
            (area.right, area.bottom),
            (area.right, area.top),
            (area.left, area.top),
        ];
        let at = writer.pos();
        let mut target = if at == corners[0] {
            corners[1]
        } else if at == corners[1] {
            corners[2]
        } else if at == corners[2] {
            corners[3]
        } else {
            corners[0]
        };
        if wipe.len() == 2 && wipe[0] == at {
            target = wipe[1];
        }
        writer.add_wipe_point(at).add_wipe_point(target);
        writer.append("; WIPE_TOWER_END\n");
        Ok(self.block_visit(writer, filament))
    }

    /// `construct_block_tcr`.
    fn block_visit(&self, writer: Writer, filament: usize) -> Visit {
        Visit {
            start_pos: writer.start_rotated(),
            end_pos: writer.pos_rotated(),
            initial_tool: filament,
            new_tool: filament,
            print_z: self.z,
            layer_height: self.height,
            wipe_path: writer.wipe_path.clone(),
            is_finish_first: true,
            gcode: writer.gcode,
            ..Visit::default()
        }
    }

    /// `finish_block_solid`: the rest of a block on a contact or solid
    /// layer, solid from the corner the layer picks; a block over a sparse
    /// layer at the bridge flow of a 0.2 mm layer (not on the layer above
    /// the first, which stands on the solid first layer).
    fn finish_block_solid(
        &mut self,
        block: usize,
        filament: usize,
        kind: LayerType,
    ) -> Result<Visit, PlanError> {
        let pw = self.pw();
        let planned = &self.plan.blocks[block];
        let mut height = self.height;
        let mut e_flow = self.flow;
        if self.index > 1
            && planned.layer_types[self.index - 1] == LayerType::Normal
            && self.flow < self.flow_at(0.2)
        {
            height = 0.2;
            e_flow = self.flow_at(0.2);
        }
        let mut writer = self.writer(height, e_flow, filament);
        writer.append("; WIPE_TOWER_START\n");
        let mut feedrate = self.tower_feedrate(filament, self.is_first_layer());
        if matches!(kind, LayerType::Contact | LayerType::ContactUp) {
            feedrate = CONTACT_SPEED;
        }
        let cur = self.blocks[block].cur_depth;
        let fill = Box2::new(
            pw,
            cur,
            self.width() - 2.0 * pw,
            planned.start_depth + planned.layer_depths[self.index] - cur - pw,
        );
        let dy = fill.top - fill.bottom;
        let n = ((dy + 0.25 * pw) / pw + 1.0) as i32;
        let spacing = pw;
        let grid_depth = spacing * (n - 1) as f32;
        let (initial, up_to_down) = match self.index % 4 {
            0 => {
                self.left_to_right = true;
                ((fill.left, fill.bottom), false)
            }
            1 => {
                self.left_to_right = false;
                ((fill.right, fill.bottom + grid_depth), true)
            }
            2 => {
                self.left_to_right = false;
                ((fill.right, fill.bottom), false)
            }
            _ => {
                self.left_to_right = true;
                ((fill.left, grid_depth + fill.bottom), true)
            }
        };
        let (left, right) = (fill.left, fill.right);
        writer.append(";--------------------\n; CP EMPTY GRID START\n");
        writer.comment_with_value(" layer #", self.layer_changes + 1);
        writer.set_initial_position(initial, self.width(), self.depth());
        for i in 0..n {
            let y = writer.y();
            writer.extrude((if self.left_to_right { right } else { left }, y), feedrate);
            if i == n - 1 {
                let at = writer.pos();
                writer
                    .add_wipe_point(at)
                    .add_wipe_point((if self.left_to_right { left } else { right }, at.1));
                break;
            }
            self.left_to_right = !self.left_to_right;
            let (x, y) = (writer.x(), writer.y());
            writer.extrude(
                (x, y + spacing * if up_to_down { -1.0 } else { 1.0 }),
                feedrate,
            );
        }
        writer.append("; CP EMPTY GRID END\n;------------------\n\n\n\n\n\n\n");
        writer.append("; WIPE_TOWER_END\n");
        Ok(self.block_visit(writer, filament))
    }

    /// `merge_tcr`: one visit made of two parts, with the travel between.
    fn merge(&self, first: &Visit, second: &Visit) -> Visit {
        let mut out = first.clone();
        let (dx, dy) = (
            first.end_pos.0 - second.start_pos.0,
            first.end_pos.1 - second.start_pos.1,
        );
        if (dx * dx + dy * dy).sqrt() > EPSILON {
            let in_gcode = second.is_tool_change
                && (second.start_pos.0 - second.tool_change_start_pos.0).abs() < EPSILON
                && (second.start_pos.1 - second.tool_change_start_pos.1).abs() < EPSILON;
            if !in_gcode {
                out.gcode += &format!(
                    "G1 X{} Y{} F{}\n",
                    fixed(f64::from(second.start_pos.0), 3),
                    fixed(f64::from(second.start_pos.1), 3),
                    fixed(f64::from(self.settings.max_speed), 6)
                );
            }
        }
        out.gcode += &second.gcode;
        out.end_pos = second.end_pos;
        out.wipe_path = second.wipe_path.clone();
        out.initial_tool = first.initial_tool;
        out.new_tool = second.new_tool;
        out.is_contact = first.is_contact || second.is_contact;
        if first.is_tool_change {
            out.is_tool_change = true;
            out.tool_change_start_pos = first.tool_change_start_pos;
        } else if second.is_tool_change {
            out.is_tool_change = true;
            out.tool_change_start_pos = second.tool_change_start_pos;
            out.reheats_after_cooling = second.reheats_after_cooling;
        } else {
            out.is_tool_change = false;
        }
        out.purge_volume += second.purge_volume;
        out
    }
}

/// The outline as the reference's offset takes it (clipper.cpp
/// `ClipperOffset::AddPath` with `ShortestEdgeLength`, ClipperUtils.cpp
/// `raw_offset`): corners closer than 0.005 of the offset to the last one
/// kept are dropped, so a corner of the box poking a unit or two out of a
/// rib's end grows no square of its own.
fn without_short_edges(outline: &Polygon, delta: f32) -> Polygon {
    let shortest = (f64::from(delta) * 1e6 * 0.005).abs();
    let shortest2 = shortest * shortest;
    let points = &outline.points;
    if points.is_empty() {
        return outline.clone();
    }
    let close = |a: edge_geom::Point2, b: edge_geom::Point2| {
        let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
        dx * dx + dy * dy < shortest2
    };
    let mut high = points.len() - 1;
    while high > 0 && close(points[high], points[0]) {
        high -= 1;
    }
    let mut kept = vec![points[0]];
    for &p in &points[1..=high] {
        if !close(p, *kept.last().unwrap_or(&p)) {
            kept.push(p);
        }
    }
    Polygon { points: kept }
}

/// `get_wall_filament_for_all_layer`: the filament that lays the wall
/// where it can, one of the category used on the most layers.
fn wall_filament(plan: &Plan, settings: &Settings) -> Option<usize> {
    use std::collections::BTreeMap;
    let mut categories: BTreeMap<i32, i32> = BTreeMap::new();
    let mut filaments: BTreeMap<usize, i32> = BTreeMap::new();
    let mut tool = plan.start_tool;
    for layer in &plan.layers {
        if let Some(first) = layer.changes.first() {
            tool = first.old;
        }
        if layer.changes.is_empty() {
            *filaments.entry(tool).or_default() += 1;
            *categories.entry(settings.category(tool)).or_default() += 1;
            continue;
        }
        let mut used_tools = Vec::new();
        let mut used_categories = Vec::new();
        for (i, change) in layer.changes.iter().enumerate() {
            if i == 0 {
                *filaments.entry(change.old).or_default() += 1;
                *categories.entry(settings.category(change.old)).or_default() += 1;
                used_tools.push(change.old);
                used_categories.push(settings.category(change.old));
            }
            let category = settings.category(change.new);
            if !used_categories.contains(&category) {
                *categories.entry(category).or_default() += 1;
            }
            if !used_tools.contains(&change.new) {
                *filaments.entry(change.new).or_default() += 1;
            }
            used_tools.push(change.new);
            used_categories.push(category);
        }
        tool = layer.changes.last().map_or(tool, |c| c.new);
    }
    let mut selected = (None, 0);
    for (category, count) in &categories {
        if *count > selected.1 {
            selected = (Some(*category), *count);
        }
    }
    let mut best = (None, 0);
    for (filament, count) in &filaments {
        if Some(settings.category(*filament)) == selected.0 && *count > best.1 {
            best = (Some(*filament), *count);
        }
    }
    best.0
}

/// Lays the planned tower (WipeTower.cpp `generate`, the layers' part):
/// for each layer, its visits in the order they are made.
pub fn lay(plan: &Plan, settings: &Settings) -> Result<Vec<Vec<Visit>>, PlanError> {
    // A smooth timelapse lays the tower's wall on every layer, on its own
    // before the layer's changes, and the changes lay none
    // (`only_generate_wall`).
    let only_wall = settings.smooth_timelapse;
    let area0 = settings.filaments.first().map_or(1.0, |f| f.area);
    let max_e_speed: Vec<f32> = settings
        .filaments
        .iter()
        .map(|f| {
            if f.max_volumetric_speed != 0.0 {
                f.max_volumetric_speed / area0
            } else {
                f32::MAX
            }
        })
        .collect();
    let count = plan.layers.len();
    let mut state = Layer {
        plan,
        settings,
        blocks: plan
            .blocks
            .iter()
            .map(|b| BlockState {
                cur_depth: b.start_depth,
                finish_depth: vec![0.0; count],
                last_filament_change: None,
            })
            .collect(),
        current_tool: plan.start_tool,
        left_to_right: true,
        tool_changes: 0,
        layer_changes: 0,
        index: 0,
        z: 0.0,
        height: 0.0,
        flow: 0.0,
        y_shift: 0.0,
        cur_block: None,
        max_e_speed,
    };
    let wall = wall_filament(plan, settings);
    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let layer = &plan.layers[index];
        for (block, planned) in state.blocks.iter_mut().zip(&plan.blocks) {
            block.cur_depth = planned.start_depth;
            block.last_filament_change = None;
        }
        state.index = index;
        state.z = layer.z;
        state.height = layer.height;
        state.flow = state.flow_at(layer.height);
        state.layer_changes += 1;
        if layer.depth < plan.line_width {
            result.push(Vec::new());
            continue;
        }
        state.y_shift = plan.y_shifts[index];
        // The filament that lays the layer's wall (`get_wall_filament_for_this_layer`).
        let wall_idx = if layer.changes.is_empty() {
            None
        } else if let Some(wall) = wall {
            let mut candidate = None;
            let mut found = None;
            for (i, change) in layer.changes.iter().enumerate() {
                if i == 0 {
                    if change.old == wall {
                        found = Some(wall);
                        break;
                    } else if settings.category(change.old) == settings.category(wall) {
                        candidate = Some(change.old);
                    }
                }
                if change.new == wall {
                    found = Some(wall);
                    break;
                }
                if candidate.is_none() && settings.category(change.new) == settings.category(wall) {
                    candidate = Some(change.new);
                }
            }
            found.or(candidate).or(Some(layer.changes[0].new))
        } else {
            Some(layer.changes[0].new)
        };
        let mut wall_idx = wall_idx;
        let mut finish: Option<Visit> = None;
        let mut timelapse_wall: Option<Visit> = None;
        if wall_idx.is_none() {
            let solid = plan
                .blocks
                .iter()
                .any(|b| b.layer_types[index] != LayerType::Normal);
            if solid {
                wall_idx = Some(state.current_tool);
            } else {
                if only_wall {
                    timelapse_wall = Some(state.only_wall()?);
                }
                finish = Some(state.finish_layer(!only_wall, true, true)?);
                for (block, planned) in state.blocks.iter_mut().zip(&plan.blocks) {
                    block.finish_depth[index] = planned.start_depth;
                }
            }
        }
        if wall_idx.is_some() && only_wall {
            timelapse_wall = Some(state.only_wall()?);
        }
        let mut visits: Vec<Visit> = Vec::new();
        let mut insert_at: Option<usize> = None;
        for (i, change) in layer.changes.iter().enumerate() {
            let wall_by_first = i == 0 && Some(change.old) == wall_idx;
            if wall_by_first {
                finish = Some(state.finish_layer(!only_wall, false, false)?);
            }
            let solid = state
                .block_of(change.new)
                .is_some_and(|b| plan.blocks[b].layer_types[index] == LayerType::Contact);
            visits.push(state.tool_change(change.new, solid)?);
            if !wall_by_first && Some(change.new) == wall_idx {
                finish = Some(state.finish_layer(!only_wall, false, false)?);
                insert_at = Some(i);
            }
        }
        if let Some(wall_idx) = wall_idx {
            if layer.changes.is_empty() {
                finish = Some(state.finish_layer(!only_wall, false, false)?);
            }
            for b in 0..plan.blocks.len() {
                let planned = &plan.blocks[b];
                state.blocks[b].finish_depth[index] = planned.start_depth + planned.depth;
                if state.blocks[b].cur_depth + EPSILON
                    >= planned.start_depth + planned.layer_depths[index] - plan.line_width
                {
                    continue;
                }
                let kind = planned.layer_types[index];
                let mut filament = state.blocks[b].last_filament_change;
                if let Some(first) = layer.changes.first()
                    && filament.is_none()
                    && state.block_of(first.old) == Some(b)
                {
                    filament = Some(first.old);
                }
                let filament = filament.unwrap_or(wall_idx);
                let part = if kind != LayerType::Normal {
                    let part = state.finish_block_solid(b, filament, kind)?;
                    state.blocks[b].finish_depth[index] = planned.start_depth + planned.depth;
                    part
                } else {
                    let part = state.finish_block(b, filament)?;
                    state.blocks[b].finish_depth[index] = state.blocks[b].cur_depth;
                    part
                };
                let mut inserted = false;
                if let Some(at) = visits.iter().position(|v| v.new_tool == filament) {
                    visits[at] = state.merge(&visits[at], &part);
                    inserted = true;
                }
                if state.blocks[b].last_filament_change.is_none()
                    && !inserted
                    && let Some(at) = visits.iter().position(|v| v.initial_tool == filament)
                {
                    visits[at] = state.merge(&part, &visits[at]);
                    inserted = true;
                }
                if !inserted && !part.gcode.is_empty() {
                    let base = finish.take().unwrap_or_default();
                    finish = Some(state.merge(&base, &part));
                }
            }
        }
        let finish = finish.unwrap_or_default();
        if visits.is_empty() {
            visits.push(finish);
        } else if has_moves(&finish.gcode) {
            match insert_at {
                None => visits[0] = state.merge(&finish, &visits[0]),
                Some(at) => visits[at] = state.merge(&visits[at], &finish),
            }
        }
        if let Some(wall) = timelapse_wall.filter(|wall| !wall.gcode.is_empty()) {
            visits.insert(0, wall);
        }
        result.push(visits);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use edge_config::Settings as Config;

    use super::super::plan::{ChangeIn, LayerIn, plan};
    use super::*;

    fn settings(extra: &str) -> Settings {
        let json = format!(
            r#"{{"printer_model": "Bambu Lab A1", "prime_tower_width": "35",
                "filament_type": ["PLA", "PLA"], "filament_diameter": ["1.75", "1.75"],
                "filament_max_volumetric_speed": ["21", "21"], "initial_layer_speed": "50",
                "nozzle_temperature": ["220", "220"]{extra}}}"#
        );
        Settings::from_settings(&Config::from_json(&json).unwrap(), 0).unwrap()
    }

    fn layers(count: usize) -> Vec<LayerIn> {
        (0..count)
            .map(|index| LayerIn {
                z: 0.2 * (index + 1) as f32,
                height: 0.2,
                changes: vec![ChangeIn {
                    old: index % 2,
                    new: 1 - index % 2,
                    purge_volume: 100.0,
                }],
            })
            .collect()
    }

    /// The purge rows' feeds of the first tool change on layer `layer`.
    fn row_feeds(visits: &[Vec<Visit>], layer: usize) -> Vec<i32> {
        let change = visits[layer].iter().find(|v| v.is_tool_change).unwrap();
        let wipe = change.gcode.split("; CP TOOLCHANGE WIPE").nth(1).unwrap();
        let wipe = wipe.split("; WIPE_TOWER_END").next().unwrap();
        let mut feeds: Vec<i32> = wipe
            .lines()
            .filter(|l| l.starts_with("G1 X") && l.contains(" E") && l.contains(" F"))
            .filter_map(|l| l.split(" F").nth(1).and_then(|f| f.parse().ok()))
            .collect();
        // (The ironing at the start runs at the first row's speed.)
        feeds.dedup();
        feeds
    }

    #[test]
    fn each_layer_lays_its_wall_and_the_change_its_purge() {
        let s = settings("");
        let planned = plan(&s, &layers(20), 0).unwrap();
        let visits = lay(&planned, &s).unwrap();
        assert_eq!(visits.len(), 20);
        for layer in &visits {
            assert_eq!(
                layer.len(),
                1,
                "one visit a layer: the wall merged into the change"
            );
            let visit = &layer[0];
            assert!(visit.is_tool_change);
            assert!(visit.gcode.contains("[change_filament_gcode]"));
            assert!(visit.gcode.contains("; CP TOOLCHANGE WIPE"));
        }
        // The first layer's brim: 7 rectangles round the wall, 4 lines each.
        let first = &visits[0][0].gcode;
        assert!(first.matches("\nG1 X").count() > 28);
        // What the purge lays above the first layer is its prime volume,
        // 45 mm³ (in whole rows: at least that).
        let area = std::f32::consts::PI * 0.875 * 0.875;
        let laid: f32 = visits[1][0]
            .gcode
            .split("; CP TOOLCHANGE WIPE")
            .nth(1)
            .unwrap()
            .split("; WIPE_TOWER_END")
            .next()
            .unwrap()
            .lines()
            .filter_map(|l| l.split(" E").nth(1))
            .filter_map(|e| e.split(' ').next().and_then(|e| e.parse::<f32>().ok()))
            .filter(|e| *e > 0.0)
            .sum();
        assert!(laid * area > 44.0 && laid * area < 52.0, "{}", laid * area);
    }

    #[test]
    fn the_rows_ramp_up_and_hold_the_fifth_step() {
        let s = settings("");
        let planned = plan(&s, &layers(20), 0).unwrap();
        // 21 mm³/s at 0.0914 mm³/mm is past the 90 mm/s cap: 5400 mm/min,
        // which the rows never reach (Bambu Studio's map).
        let feeds = row_feeds(&lay(&planned, &s).unwrap(), 1);
        assert_eq!(feeds, [1782, 2025, 2473, 4725, 4775]);
    }

    #[test]
    fn a_wall_is_cut_open_where_a_purge_comes_in() {
        let square = Polygon {
            points: [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]
                .map(|(x, y)| edge_geom::Point2::from_mm(x, y))
                .to_vec(),
        };
        let pieces = cut_wall_gaps(
            &square,
            &[GapPoint {
                pos: (0.0, 5.0),
                is_left: true,
            }],
            1.25,
        );
        let length: f64 = pieces
            .iter()
            .flat_map(|piece| piece.windows(2))
            .map(|w| {
                edge_geom::unscaled(w[1].x - w[0].x).hypot(edge_geom::unscaled(w[1].y - w[0].y))
            })
            .sum();
        assert!((length - 37.5).abs() < 1e-3, "{length}");
        let ends: Vec<_> = pieces
            .iter()
            .flat_map(|p| [p[0], p[p.len() - 1]])
            .map(|p| {
                (
                    (edge_geom::unscaled(p.x) * 100.0).round(),
                    (edge_geom::unscaled(p.y) * 100.0).round(),
                )
            })
            .collect();
        assert!(
            ends.contains(&(0.0, 375.0)) && ends.contains(&(0.0, 625.0)),
            "{ends:?}"
        );
    }

    #[test]
    fn a_category_that_starts_higher_up_is_laid_solid_and_slow_where_it_starts() {
        let s = Settings::from_settings(
            &Config::from_json(
                r#"{"printer_model": "Bambu Lab A1", "prime_tower_width": "35",
                    "filament_type": ["PLA", "PLA", "PETG"],
                    "filament_diameter": ["1.75", "1.75", "1.75"]}"#,
            )
            .unwrap(),
            0,
        )
        .unwrap();
        // PLA changes throughout; PETG comes in at layer 10.
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
                    vec![change(index % 2, 1 - index % 2)]
                } else {
                    vec![change(index % 2, 2), change(2, 1 - index % 2)]
                },
            })
            .collect();
        let planned = plan(&s, &input, 0).unwrap();
        let petg = planned.blocks.iter().find(|b| b.category == 300).unwrap();
        assert_eq!(petg.layer_types[10], LayerType::Contact);
        let visits = lay(&planned, &s).unwrap();
        // Below it the PETG block is filled solid by a PLA ("Solid"), and the
        // contact layer's purge ramps up towards 20 mm/s (its held fifth
        // step is 1100 mm/min).
        assert!(
            visits[10]
                .iter()
                .any(|v| v.is_contact && v.gcode.contains(" F1100"))
        );
        assert!(
            visits[9]
                .iter()
                .any(|v| v.gcode.contains("CP EMPTY GRID START"))
        );
    }

    #[test]
    fn a_smooth_timelapse_lays_the_wall_on_its_own_first() {
        let s = settings(r#", "timelapse_type": "1""#);
        let planned = plan(&s, &layers(4), 0).unwrap();
        let visits = lay(&planned, &s).unwrap();
        for layer in &visits {
            assert_eq!(layer.len(), 2, "the wall, then the change");
            assert!(!layer[0].is_tool_change && layer[1].is_tool_change);
            assert!(layer[0].gcode.contains("; WIPE_TOWER_START"));
        }
    }
}
