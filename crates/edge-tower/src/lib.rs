//! The prime tower: a block beside the print where a nozzle just brought
//! into use lays down its first plastic, so that what oozed or dried while
//! it waited does not go into the part.
//!
//! A first working version after the reference's WipeTower2.cpp, for a
//! printer with a nozzle per filament, where nothing has to be flushed
//! between colours and every tool change primes the same `prime_volume`.
//! Kept from the reference: the line width, how deep a block a tool change
//! takes, that the tower stands on every layer up to the last tool change
//! and is never deeper above than below, a solid first layer, the lines
//! that carry the next layer over depth no tool change uses, the ribbed
//! wall (a tower with one is made as wide as it is deep, has a rib along
//! each diagonal that runs out past the corners, longest on the bed and
//! gone at the top, and stands where the ribs' ends put it), and the brim,
//! which follows the wall and steps in a loop a layer above the first.
//!
//! Lengths here are the reference's own: the wall's middle runs from the
//! tower's corner, so the tower's outside is half a line beyond it.
//!
//! Not done: the cone wall, the gap in the wall where a nozzle comes in,
//! the slow ironing pass that starts a block, turning the tower end for
//! end on alternate layers, ramming, and leaving out or thinning the
//! layers no tool change uses.

use std::f64::consts::PI;

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionPath, Flow, FlowError, Role};
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};

/// The tower's lines are this many nozzle diameters wide
/// (WipeTower2.hpp `Width_To_Nozzle_Ratio`).
const WIDTH_TO_NOZZLE: f64 = 1.25;
/// The reference plans a block as if two lines this many times wider than
/// the rest ran down its sides (`ramming_line_width_ratio`).
const RAMMING_WIDTH: f64 = 2.0;

/// A rib's corners, and where it leaves the tower's sides, are rounded to
/// arcs of this many mm (WipeTower2.cpp `rounding_polygon`), each drawn
/// in this many pieces (`arc_fit_size`).
const ROUNDING: f64 = 2.0;
const ARC_PIECES: usize = 20;

/// What stands round the tower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wall {
    /// A plain box.
    Box,
    /// A box with a rib along each diagonal (`wipe_tower_wall_type` at
    /// `rib`).
    Ribs {
        /// How wide a rib is (`wipe_tower_rib_width`).
        width: f64,
        /// How much longer than the tower's diagonal a rib is on the bed,
        /// both ends together (`wipe_tower_extra_rib_length`).
        extra: f64,
        /// Whether its corners are rounded (`wipe_tower_fillet_wall`).
        rounded: bool,
    },
}

/// What can go wrong planning a tower.
#[derive(Debug, thiserror::Error)]
pub enum TowerError {
    #[error(transparent)]
    Flow(#[from] FlowError),
    #[error(transparent)]
    Clip(#[from] ClipError),
}

/// The settings the tower depends on. Defaults are the reference's
/// (PrintConfig.cpp). Lengths are in mm.
#[derive(Clone, Debug, PartialEq)]
pub struct TowerSettings {
    pub enabled: bool,
    /// The front left corner of the ground the wall takes on the bed,
    /// ribs included, measured to the middle of the wall's line.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    /// How much plastic a nozzle lays down after a tool change, in mm³.
    pub prime_volume: f64,
    pub brim_width: f64,
    pub nozzle_diameter: f64,
    /// How far apart a block's rows are, in line widths, above the first layer.
    pub extra_spacing: f64,
    /// How much more plastic than usual a block's lines carry.
    pub extra_flow: f64,
    /// The furthest apart the lines across unused depth may be, since the
    /// layer above is laid over them.
    pub bridging: f64,
    pub wall: Wall,
    /// Whether the brim goes on above the first layer, a loop narrower
    /// each layer (`prime_tower_brim_chamfer`), and from how wide at most.
    pub brim_chamfer: bool,
    pub brim_chamfer_max: f64,
    /// Settings present in the input that are not honoured, each with what
    /// is done instead.
    pub unsupported: Vec<String>,
}

impl Default for TowerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            x: 15.0,
            y: 220.0,
            width: 60.0,
            prime_volume: 45.0,
            brim_width: 3.0,
            nozzle_diameter: 0.4,
            extra_spacing: 1.0,
            extra_flow: 1.0,
            bridging: 10.0,
            wall: Wall::Box,
            brim_chamfer: true,
            brim_chamfer_max: 4.0,
            unsupported: Vec::new(),
        }
    }
}

impl TowerSettings {
    /// Reads what is present; anything absent keeps its default. The
    /// tower's place is kept per plate, and `plate` picks which.
    pub fn from_settings(settings: &Settings, plate: usize) -> Result<Self, ConfigError> {
        let defaults = Self::default();
        let number = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(settings.float(key)?.unwrap_or(default))
        };
        let placed = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(settings
                .floats(key)?
                .and_then(|values| values.get(plate).or(values.last()).copied().flatten())
                .unwrap_or(default))
        };
        let share = |key: &str, default: f64| -> Result<f64, ConfigError> {
            Ok(match settings.float_or_percent(key)? {
                Some(FloatOrPercent::Percent(percent) | FloatOrPercent::Value(percent)) => {
                    percent / 100.0
                }
                None => default,
            })
        };
        let enabled = settings
            .flag("enable_prime_tower")?
            .unwrap_or(defaults.enabled);
        let mut unsupported = Vec::new();
        if enabled {
            for (key, plain, instead) in [
                (
                    "wipe_tower_wall_gap",
                    "0",
                    "the wall is left whole where a nozzle comes into the tower",
                ),
                (
                    "wipe_tower_rotation_angle",
                    "0",
                    "the tower stands square to the bed",
                ),
                (
                    "wipe_tower_no_sparse_layers",
                    "0",
                    "the tower is printed on every layer up to the last tool change",
                ),
            ] {
                if let Some(value) = settings.text(key)
                    && value != plain
                {
                    unsupported.push(format!("{key} \"{value}\": {instead}"));
                }
            }
            if let Some(kind) = settings.text("wipe_tower_wall_type")
                && !matches!(kind, "rectangle" | "rib")
            {
                unsupported.push(format!(
                    "wipe_tower_wall_type \"{kind}\": the tower has a plain box for a wall"
                ));
            }
            if settings
                .texts("filament_multitool_ramming")
                .is_some_and(|values| values.iter().any(|value| value == "1"))
            {
                unsupported.push(
                    "filament_multitool_ramming: a filament is not rammed before it is put away"
                        .into(),
                );
            }
        }
        Ok(Self {
            enabled,
            x: placed("wipe_tower_x", defaults.x)?,
            y: placed("wipe_tower_y", defaults.y)?,
            width: number("prime_tower_width", defaults.width)?,
            prime_volume: number("prime_volume", defaults.prime_volume)?,
            brim_width: number("prime_tower_brim_width", defaults.brim_width)?.max(0.0),
            nozzle_diameter: settings
                .floats("nozzle_diameter")?
                .and_then(|nozzles| nozzles.first().copied().flatten())
                .unwrap_or(defaults.nozzle_diameter),
            extra_spacing: share("wipe_tower_extra_spacing", defaults.extra_spacing)?.max(0.5),
            extra_flow: share("wipe_tower_extra_flow", defaults.extra_flow)?.max(0.1),
            bridging: number("wipe_tower_bridging", defaults.bridging)?.max(1.0),
            wall: if settings.text("wipe_tower_wall_type") == Some("rib") {
                Wall::Ribs {
                    width: number("wipe_tower_rib_width", 8.0)?.max(0.0),
                    extra: number("wipe_tower_extra_rib_length", 0.0)?.max(0.0),
                    rounded: settings.flag("wipe_tower_fillet_wall")?.unwrap_or(true),
                }
            } else {
                Wall::Box
            },
            brim_chamfer: settings
                .flag("prime_tower_brim_chamfer")?
                .unwrap_or(defaults.brim_chamfer),
            brim_chamfer_max: number(
                "prime_tower_brim_chamfer_max_width",
                defaults.brim_chamfer_max,
            )?
            .max(0.0),
            unsupported,
        })
    }

    /// A tower line on a layer `height` mm thick.
    pub fn flow(&self, height: f64) -> Result<Flow, FlowError> {
        Flow::new(
            WIDTH_TO_NOZZLE * self.nozzle_diameter,
            height,
            self.nozzle_diameter,
        )
    }

    /// How much of the tower's depth one tool change takes on a layer
    /// `height` mm thick (WipeTower2.cpp `get_wipe_depth`): enough rows,
    /// across a width a little less than the tower's, to lay down
    /// `prime_volume`.
    fn block_depth(&self, width: f64, height: f64, first_layer: bool) -> f64 {
        let line = WIDTH_TO_NOZZLE * self.nozzle_diameter;
        let per_mm = height * (line - height * (1.0 - PI / 4.0));
        let length = self.prime_volume / per_mm / self.extra_flow;
        let across = (width - line - 2.0 * line * RAMMING_WIDTH).max(line);
        let rows = (length / across).floor() + 1.0;
        rows * line * self.row_step(first_layer)
    }

    /// How far apart a block's rows are, in line widths.
    fn row_step(&self, first_layer: bool) -> f64 {
        if first_layer {
            self.extra_flow
        } else {
            self.extra_spacing * self.extra_flow
        }
    }
}

/// What the tower has on one layer.
#[derive(Clone, Debug, Default)]
pub struct TowerLayer {
    /// What fills the depth no tool change uses, the wall, and the brim.
    /// Printed once, by whichever nozzle comes first.
    pub finish: Vec<ExtrusionPath>,
    /// For each tool change on the layer, in order, what the new nozzle
    /// lays down.
    pub changes: Vec<Vec<ExtrusionPath>>,
}

/// The whole tower.
#[derive(Clone, Debug, Default)]
pub struct Tower {
    /// One entry for each layer of the print; empty above the last tool change.
    pub layers: Vec<TowerLayer>,
    /// The ground the tower takes, ribs and brim included, as min x, min
    /// y, max x, max y in mm; none when there is no tower.
    pub footprint: Option<[f64; 4]>,
}

/// The least depth a tower as tall as `height` mm should have to stand
/// (WipeTower.cpp `get_limit_depth_by_height`): 20 mm up to 100 mm tall,
/// 40 mm from 250 mm, and in proportion between.
fn least_depth(height: f64) -> f64 {
    if height <= 100.0 {
        20.0
    } else if height >= 250.0 {
        40.0
    } else {
        20.0 + (height - 100.0) / 150.0 * 20.0
    }
}

/// A closed outline with every corner that turns between 30 and 150
/// degrees replaced by an arc that leaves each side `ROUNDING` mm from
/// the corner, or less on a short side (WipeTower2.cpp `rounding_polygon`).
fn rounded(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let count = points.len();
    if count < 3 {
        return points.to_vec();
    }
    let tolerance = 30f64.to_radians().cos();
    let mut out = Vec::with_capacity(count * 2);
    for i in 0..count {
        let (a, b, c) = (
            points[(i + count - 1) % count],
            points[i],
            points[(i + 1) % count],
        );
        let (before, after) = ((b.0 - a.0).hypot(b.1 - a.1), (c.0 - b.0).hypot(c.1 - b.1));
        if before == 0.0 || after == 0.0 {
            out.push(b);
            continue;
        }
        let (ab, bc) = (
            ((b.0 - a.0) / before, (b.1 - a.1) / before),
            ((c.0 - b.0) / after, (c.1 - b.1) / after),
        );
        let cos = (ab.0 * bc.0 + ab.1 * bc.1).clamp(-1.0, 1.0);
        if cos.abs() >= tolerance {
            out.push(b);
            continue;
        }
        let left_turn = ab.0 * bc.1 - ab.1 * bc.0 > 0.0;
        let reach = ROUNDING.min(before / 2.1).min(after / 2.1);
        let from = (b.0 - ab.0 * reach, b.1 - ab.1 * reach);
        let to = (b.0 + bc.0 * reach, b.1 + bc.1 * reach);
        // The arc's middle lies on the corner's bisector.
        let chord = (to.0 - from.0).hypot(to.1 - from.1);
        let mut inward = (-(to.1 - from.1) / chord, (to.0 - from.0) / chord);
        if !left_turn {
            inward = (-inward.0, -inward.1);
        }
        let away = reach / (cos.acos() / 2.0).sin();
        let middle = (b.0 + inward.0 * away, b.1 + inward.1 * away);
        let radius = (from.0 - middle.0).hypot(from.1 - middle.1);
        let start = (from.1 - middle.1).atan2(from.0 - middle.0);
        let mut sweep = (to.1 - middle.1).atan2(to.0 - middle.0) - start;
        if left_turn {
            while sweep <= 0.0 {
                sweep += 2.0 * PI;
            }
        } else {
            while sweep >= 0.0 {
                sweep -= 2.0 * PI;
            }
        }
        for piece in 0..ARC_PIECES {
            let angle = start + sweep * piece as f64 / ARC_PIECES as f64;
            out.push((
                middle.0 + radius * angle.cos(),
                middle.1 + radius * angle.sin(),
            ));
        }
        out.push(to);
    }
    out.dedup();
    out
}

fn region(points: &[(f64, f64)]) -> ExPolygon {
    ExPolygon {
        contour: Polygon {
            points: points
                .iter()
                .map(|(x, y)| Point2::from_mm(*x, *y))
                .collect(),
        },
        holes: Vec::new(),
    }
}

/// The outline of what several shapes cover together, in mm.
fn outline_of(shapes: &[ExPolygon]) -> Result<Vec<(f64, f64)>, ClipError> {
    Ok(edge_clip::union_ex(shapes)?
        .first()
        .map(|whole| {
            whole
                .contour
                .points
                .iter()
                .map(|p| (unscaled(p.x), unscaled(p.y)))
                .collect()
        })
        .unwrap_or_default())
}

/// How the tower is laid out, in mm from the corner of its wall: the
/// reference's `m_wipe_tower_width`, `m_wipe_tower_depth` and
/// `m_wipe_tower_height`.
struct Shape {
    /// From the middle of one side wall to the middle of the other.
    span: f64,
    /// What the busiest layer's tool changes take, front to back.
    blocks: f64,
    /// The top of the tower's last layer.
    height: f64,
    line: f64,
    wall: Wall,
}

impl Shape {
    /// The middle of the wall's line on the layer whose top is `z` mm
    /// up (WipeTower2.cpp `support_rib_wall_polygon`).
    fn wall(&self, z: f64) -> Result<Vec<(f64, f64)>, ClipError> {
        let (span, back) = (self.span, self.blocks + self.line);
        let plain = [(0.0, 0.0), (span, 0.0), (span, back), (0.0, back)];
        let Wall::Ribs {
            width,
            extra,
            rounded: round,
        } = self.wall
        else {
            return Ok(plain.to_vec());
        };
        // A rib is at least as long as the diagonal it lies on; a tower
        // too slight for its height gets its ribs lengthened, not its
        // depth (`generate`).
        let diagonal = span.hypot(self.blocks);
        let mut length = 0.0f64;
        let least = least_depth(self.height);
        if self.blocks + 1e-4 < least {
            length = least * std::f64::consts::SQRT_2;
        }
        let length = (length.max(diagonal) + extra).max(diagonal);
        // Past each corner by half of what is over, less the higher up.
        let past = (length - diagonal) / 2.0 * (self.height - z).abs() / self.height;
        let half = width.min(self.blocks.min(span) / 2.0) / 2.0;
        let mut shapes = Vec::with_capacity(3);
        for (from, to) in [
            ((0.0, 0.0), (span, self.blocks)),
            ((span, 0.0), (0.0, self.blocks)),
        ] {
            let along = ((to.0 - from.0) / diagonal, (to.1 - from.1) / diagonal);
            let across = (-along.1 * half, along.0 * half);
            let (a, b) = (
                (from.0 - along.0 * past, from.1 - along.1 * past),
                (to.0 + along.0 * past, to.1 + along.1 * past),
            );
            shapes.push(region(&[
                (a.0 + across.0, a.1 + across.1),
                (a.0 - across.0, a.1 - across.1),
                (b.0 - across.0, b.1 - across.1),
                (b.0 + across.0, b.1 + across.1),
            ]));
        }
        shapes.push(region(&plain));
        let ribbed = outline_of(&shapes)?;
        if !round {
            return Ok(ribbed);
        }
        outline_of(&[region(&rounded(&ribbed)), region(&plain)])
    }
}

/// Plans the tower for a print whose layers are `heights` mm thick and
/// have `changes` tool changes each.
pub fn plan(
    changes: &[usize],
    heights: &[f64],
    settings: &TowerSettings,
) -> Result<Tower, TowerError> {
    let count = changes.len();
    let mut tower = Tower {
        layers: vec![TowerLayer::default(); count],
        footprint: None,
    };
    let Some(last) = changes.iter().rposition(|changes| *changes > 0) else {
        return Ok(tower);
    };
    if !settings.enabled {
        return Ok(tower);
    }
    let line = WIDTH_TO_NOZZLE * settings.nozzle_diameter;
    // A layer is as deep as its tool changes need, and no shallower than
    // any layer above it, so nothing is laid over air.
    let blocks_at = |span: f64| {
        (0..=last)
            .map(|layer| {
                changes[layer] as f64 * settings.block_depth(span, heights[layer], layer == 0)
            })
            .fold(0.0, f64::max)
    };
    let mut span = settings.width.max(6.0 * line);
    // A tower with a ribbed wall takes the ground of the one set but as
    // a square, to the next whole line (WipeTower2.cpp `generate`).
    if matches!(settings.wall, Wall::Ribs { .. }) {
        span = ((blocks_at(span) * span).sqrt() / line).ceil() * line;
    }
    let shape = Shape {
        span,
        blocks: blocks_at(span),
        height: heights[..=last].iter().sum(),
        line,
        wall: settings.wall,
    };
    let blocks = shape.blocks;
    // The tower stands where the wall of its first layer, ribs and all,
    // has its corner on the place set (`set_rib_offset`).
    let first_wall = shape.wall(heights[0])?;
    let corner = first_wall
        .iter()
        .fold((f64::MAX, f64::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
    let at = |x: f64, y: f64| Point2::from_mm(settings.x - corner.0 + x, settings.y - corner.1 + y);

    let mut top = 0.0;
    for layer in 0..=last {
        let first = layer == 0;
        top += heights[layer];
        let flow = settings.flow(heights[layer])?;
        let path = |points: Vec<Point2>| ExtrusionPath {
            points,
            role: Role::PrimeTower,
            flow,
        };

        // Tool changes take their blocks from the front; each is rows
        // from side to side, the first of them laid slowly.
        let (left, right) = (line, span - line);
        let step = line * settings.row_step(first);
        let block = settings.block_depth(span, heights[layer], first);
        let rows = (block / step).round().max(1.0) as usize;
        let mut front = line;
        for _ in 0..changes[layer] {
            let mut points = Vec::with_capacity(2 * rows);
            for row in 0..rows {
                let y = front + row as f64 * step;
                let (from, to) = if row % 2 == 0 {
                    (left, right)
                } else {
                    (right, left)
                };
                points.push(at(from, y));
                points.push(at(to, y));
            }
            let slow: Vec<Point2> = points.drain(..2).collect();
            let mut block_paths = vec![ExtrusionPath {
                points: slow,
                role: Role::PrimeLine,
                flow,
            }];
            if points.len() >= 2 {
                block_paths.push(path(points));
            }
            tower.layers[layer].changes.push(block_paths);
            front += block;
        }

        // The depth no tool change uses (`finish_layer`): a frame, and
        // inside it lines solid on the bed, or above it from front to
        // back close enough together to carry the layer over them.
        let mut finish = Vec::new();
        let used = changes[layer] as f64 * block;
        let fill = ((blocks - used).max(0.0) - line).max(0.0);
        let (low, high) = (blocks - fill, blocks);
        if fill > line - 1e-3 {
            finish.push(path(vec![
                at(left, low),
                at(right, low),
                at(right, high),
                at(left, high),
                at(left, low),
            ]));
        }
        let room = fill - line;
        if room > line {
            let start = left + 2.0 * line;
            if first {
                // Lines that touch the frame.
                let (left, right) = (left + line, right - line);
                let count = (room / line) as usize;
                let pitch = if count > 1 {
                    (room - line) / (count - 1) as f64
                } else {
                    0.0
                };
                let mut points = vec![at(start, low)];
                let mut x = start;
                for row in 0..count {
                    let y = low + line + row as f64 * pitch;
                    points.push(at(x, y));
                    x = if row % 2 == 1 { left } else { right };
                    points.push(at(x, y));
                }
                points.push(at(x, high));
                finish.push(path(points));
            } else {
                // One line up the left, then one every so far across,
                // each from the end the last stopped at.
                finish.push(path(vec![at(start, low), at(start, high)]));
                let (left, right) = (start, right - 2.0 * line);
                let count = 1 + ((right - left) / settings.bridging) as usize;
                let pitch = (right - left) / count as f64;
                let mut from = high;
                for column in 1..=count {
                    let x = left + column as f64 * pitch;
                    let to = if column % 2 == 1 { low } else { high };
                    finish.push(path(vec![at(x, from), at(x, to)]));
                    from = to;
                }
            }
        }

        // The wall, its line's middle on the outline.
        let wall = shape.wall(top)?;
        let closed = |outline: &[(f64, f64)]| -> Vec<Point2> {
            outline
                .iter()
                .chain(outline.first())
                .map(|(x, y)| at(*x, *y))
                .collect()
        };
        finish.push(path(closed(&wall)));

        // The brim holds the tower to the bed: loops out from the wall,
        // all of them on the first layer and one fewer each layer above
        // it, from no wider than the chamfer allows.
        let spacing = flow.spacing;
        let mut loops = ((settings.brim_width + spacing / 2.0) / spacing) as usize;
        if !first {
            loops = if settings.brim_chamfer {
                loops
                    .min((settings.brim_chamfer_max / spacing) as usize)
                    .saturating_sub(layer)
            } else {
                0
            };
        }
        let mut ring = vec![region(&wall)];
        for _ in 0..loops {
            ring = edge_clip::offset(&ring, scaled(spacing), Join::MITER)?;
            ring.truncate(1);
            let Some(outer) = ring.first() else { break };
            let outline: Vec<(f64, f64)> = outer
                .contour
                .points
                .iter()
                .map(|p| (unscaled(p.x), unscaled(p.y)))
                .collect();
            finish.push(path(closed(&outline)));
            ring = vec![region(&outline)];
        }
        tower.layers[layer].finish = finish;
    }
    // The ground it takes: its first layer, out to the edge of the lines.
    let half = line / 2.0;
    tower.footprint = tower.layers[0]
        .finish
        .iter()
        .flat_map(|path| &path.points)
        .map(|p| (unscaled(p.x), unscaled(p.y)))
        .fold(None, |ground: Option<[f64; 4]>, (x, y)| {
            Some(match ground {
                None => [x, y, x, y],
                Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x), d.max(y)],
            })
        })
        .map(|[a, b, c, d]| [a - half, b - half, c + half, d + half]);
    Ok(tower)
}

#[cfg(test)]
mod tests {
    use edge_geom::unscaled;

    use super::*;

    fn on() -> TowerSettings {
        TowerSettings {
            enabled: true,
            x: 100.0,
            y: 50.0,
            width: 30.0,
            brim_width: 5.0,
            extra_spacing: 1.2,
            ..TowerSettings::default()
        }
    }

    fn extent<'a>(paths: impl IntoIterator<Item = &'a ExtrusionPath>) -> [f64; 4] {
        paths
            .into_iter()
            .flat_map(|path| &path.points)
            .map(|p| (unscaled(p.x), unscaled(p.y)))
            .fold(
                [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
                |[a, b, c, d], (x, y)| [a.min(x), b.min(y), c.max(x), d.max(y)],
            )
    }

    #[test]
    fn a_block_lays_down_the_prime_volume() {
        let settings = on();
        let tower = plan(&[0, 1, 0], &[0.2; 3], &settings).unwrap();
        let block = &tower.layers[1].changes[0];
        // One slow line first, then the rest.
        assert_eq!(
            (block[0].role, block[1].role),
            (Role::PrimeLine, Role::PrimeTower)
        );
        assert_eq!(block[0].points.len(), 2);
        let volume: f64 = block.iter().map(ExtrusionPath::volume_mm3).sum();
        // At least what was asked for, and not much more than a row over.
        assert!((45.0..45.0 * 1.2).contains(&volume), "{volume} mm³");
        // 0.5 mm lines on a 0.4 mm nozzle, rows 1.2 line widths apart.
        assert_eq!(block[0].flow.width, 0.5);
        let rows: Vec<f64> = block
            .iter()
            .flat_map(|path| &path.points)
            .map(|p| unscaled(p.y))
            .collect();
        assert!((rows[2] - rows[0] - 0.6).abs() < 1e-6, "{rows:?}");
    }

    /// The wall on a layer of a tower with no brim above the bed: the
    /// last thing laid.
    fn wall_of(tower: &Tower, layer: usize) -> &ExtrusionPath {
        tower.layers[layer].finish.last().unwrap()
    }

    #[test]
    fn the_tower_stands_on_every_layer_up_to_the_last_change() {
        let settings = TowerSettings {
            brim_chamfer: false,
            ..on()
        };
        // Changes on layers 1 and 3 of six; two on layer 3.
        let tower = plan(&[0, 1, 0, 2, 0, 0], &[0.2; 6], &settings).unwrap();
        for layer in 0..=3 {
            assert!(
                !tower.layers[layer].finish.is_empty(),
                "no tower on {layer}"
            );
        }
        assert!(tower.layers[4].finish.is_empty() && tower.layers[5].finish.is_empty());
        assert_eq!(
            tower
                .layers
                .iter()
                .map(|l| l.changes.len())
                .collect::<Vec<_>>(),
            [0, 1, 0, 2, 0, 0]
        );
        // The wall is the same on every layer: as deep as the busiest
        // needs, its line's middle from the place set.
        let wall = |layer: usize| extent([wall_of(&tower, layer)]);
        assert_eq!(wall(1), wall(3));
        let [min_x, min_y, max_x, max_y] = wall(3);
        assert!((min_x - 100.0).abs() < 1e-6 && (min_y - 50.0).abs() < 1e-6);
        assert!((max_x - 130.0).abs() < 1e-6);
        let two_blocks = 2.0 * settings.block_depth(settings.width, 0.2, false) + 0.5;
        assert!(
            (max_y - min_y - two_blocks).abs() < 1e-6,
            "{} for {two_blocks}",
            max_y - min_y
        );
        // Everything a change lays down is inside the wall.
        for layer in &tower.layers {
            for block in &layer.changes {
                let [a, b, c, d] = extent(block);
                assert!(a > min_x && b > min_y && c < max_x && d < max_y);
            }
        }
        // Where a layer uses no depth, a frame goes round it and lines
        // cross it no further apart than the layer above can bridge.
        let finish = &tower.layers[2].finish;
        assert_eq!(finish[0].points.len(), 5);
        let columns: Vec<f64> = finish[1..finish.len() - 1]
            .iter()
            .map(|line| {
                assert_eq!(line.points.len(), 2);
                assert_eq!(line.points[0].x, line.points[1].x);
                unscaled(line.points[0].x)
            })
            .collect();
        assert!(
            columns
                .windows(2)
                .all(|pair| pair[1] > pair[0] && pair[1] - pair[0] <= 10.0)
        );
        assert_eq!(columns.len(), 4);
        // Each starts at the end the last stopped at, so the nozzle
        // only moves across between them.
        for pair in finish[1..finish.len() - 1].windows(2) {
            assert_eq!(pair[0].points[1].y, pair[1].points[0].y);
        }
    }

    #[test]
    fn the_first_layer_is_solid_and_has_a_brim() {
        let settings = on();
        let tower = plan(&[0, 1], &[0.25, 0.2], &settings).unwrap();
        let first = &tower.layers[0].finish;
        // A frame, solid fill, the wall, and brim loops out to 5 mm.
        let spacing = settings.flow(0.25).unwrap().spacing;
        let loops = ((5.0 + spacing / 2.0) / spacing).floor() as usize;
        assert_eq!(first.len(), 3 + loops);
        let fill = &first[1];
        let rows: Vec<f64> = fill.points[1..fill.points.len() - 1]
            .iter()
            .step_by(2)
            .map(|p| unscaled(p.y))
            .collect();
        assert!(rows.len() > 10);
        assert!(
            rows.windows(2)
                .all(|pair| (0.5..0.56).contains(&(pair[1] - pair[0]))),
            "{rows:?}"
        );
        let [min_x, min_y, max_x, max_y] = extent(first);
        let reach = 100.0 - min_x;
        assert!(
            (reach - loops as f64 * spacing).abs() < 1e-3,
            "brim reaches {reach}"
        );
        assert!(reach > 4.5 && reach <= 5.0 + spacing / 2.0);
        // The ground it takes is known, for the skirt and the bed's
        // edge: out to the edge of the outermost line.
        let [a, b, c, d] = tower.footprint.unwrap();
        for (ground, line) in [
            (a, min_x - 0.25),
            (b, min_y - 0.25),
            (c, max_x + 0.25),
            (d, max_y + 0.25),
        ] {
            assert!((ground - line).abs() < 1e-6);
        }
    }

    #[test]
    fn the_brim_steps_in_a_loop_a_layer_above_the_first() {
        let settings = on();
        let heights = [0.2; 12];
        let mut changes = [0; 12];
        changes[11] = 1;
        let loops = |settings: &TowerSettings, layer: usize| {
            // Every closed line but the frame and the wall.
            let tower = plan(&changes, &heights, settings).unwrap();
            let closed = tower.layers[layer]
                .finish
                .iter()
                .filter(|path| path.points.first() == path.points.last())
                .count();
            closed - 2
        };
        let spacing = settings.flow(0.2).unwrap().spacing;
        // From no wider than the 4 mm the chamfer allows.
        let widest = (4.0 / spacing) as usize;
        assert_eq!(widest, 8);
        for layer in 1..=9 {
            assert_eq!(
                loops(&settings, layer),
                widest.saturating_sub(layer),
                "layer {layer}"
            );
        }
        // Each loop is a line's spacing outside the last.
        let tower = plan(&changes, &heights, &settings).unwrap();
        let finish = &tower.layers[3].finish;
        let outer = extent([&finish[finish.len() - 1]]);
        let inner = extent([&finish[finish.len() - 2]]);
        assert!((inner[0] - outer[0] - spacing).abs() < 1e-3);
        // Without the chamfer the brim is on the bed and nowhere else.
        let flat = TowerSettings {
            brim_chamfer: false,
            ..on()
        };
        assert_eq!(loops(&flat, 1), 0);
    }

    fn ribbed() -> TowerSettings {
        TowerSettings {
            wall: Wall::Ribs {
                width: 8.0,
                extra: 8.0,
                rounded: true,
            },
            brim_width: 0.0,
            ..on()
        }
    }

    #[test]
    fn ribs_run_past_the_corners_and_shorten_on_the_way_up() {
        let settings = ribbed();
        let tower = plan(&[1, 5, 5, 5, 5], &[0.2; 5], &settings).unwrap();
        // The rows give the tower's own box: a line inside each wall.
        let [left, front, right, _] = extent(&tower.layers[1].changes[0]);
        let (box_left, box_right) = (left - 0.5, right + 0.5);
        // The first layer's wall has its corner on the place set.
        let [min_x, min_y, max_x, _] = extent([wall_of(&tower, 0)]);
        assert!((min_x - 100.0).abs() < 1e-3 && (min_y - 50.0).abs() < 1e-3);
        // Each rib reaches past the box by a share of its 4 mm and of
        // half its width, both along the diagonal.
        let past = box_left - min_x;
        assert!((3.0..6.0).contains(&past), "{past}");
        assert!((max_x - box_right - past).abs() < 0.05);
        assert!(front - 0.5 - min_y > 3.0);
        // Higher up it reaches less, and the wall never comes inside
        // the box.
        let mut last = past;
        for layer in 1..5 {
            let [min_x, ..] = extent([wall_of(&tower, layer)]);
            let past = box_left - min_x;
            assert!(past < last && past >= 0.0, "layer {layer}: {past}");
            last = past;
        }
        // The same ribs unrounded reach further: their corners are whole.
        let sharp = TowerSettings {
            wall: Wall::Ribs {
                width: 8.0,
                extra: 8.0,
                rounded: false,
            },
            ..ribbed()
        };
        let tower = plan(&[1, 5, 5, 5, 5], &[0.2; 5], &sharp).unwrap();
        let wall = wall_of(&tower, 0);
        let [left, ..] = extent(&tower.layers[1].changes[0]);
        let [min_x, ..] = extent([wall]);
        assert!(left - 0.5 - min_x > past);
        // A box with two bars across it: each bar's end is two corners,
        // and it leaves the box at two more.
        assert_eq!(wall.points.len(), 16 + 1);
    }

    #[test]
    fn corners_are_rounded_to_arcs_that_stay_inside_them() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let round = rounded(&square);
        // Each corner becomes twenty pieces of arc and its far end.
        assert_eq!(round.len(), 4 * (ARC_PIECES + 1));
        for (x, y) in &round {
            assert!((-1e-9..=10.0 + 1e-9).contains(x) && (-1e-9..=10.0 + 1e-9).contains(y));
        }
        // The arc leaves each side 2 mm from the corner and is a
        // quarter of a circle of that radius.
        assert!(
            round.contains(&(8.0, 0.0))
                || round
                    .iter()
                    .any(|p| (p.0 - 8.0).abs() < 1e-9 && p.1.abs() < 1e-9)
        );
        let nearest = round
            .iter()
            .map(|(x, y)| (10.0 - x).hypot(*y))
            .fold(f64::MAX, f64::min);
        assert!(
            (nearest - 2.0 * (std::f64::consts::SQRT_2 - 1.0)).abs() < 0.01,
            "{nearest}"
        );
        // A side too short for that is rounded by less.
        let sliver = [(0.0, 0.0), (10.0, 0.0), (10.0, 1.05), (0.0, 1.05)];
        let round = rounded(&sliver);
        assert!(round.iter().all(|(_, y)| (-1e-9..=1.05 + 1e-9).contains(y)));
        // A corner that barely turns is left alone.
        let blunt = [(0.0, 0.0), (5.0, -0.1), (10.0, 0.0), (5.0, 10.0)];
        assert!(rounded(&blunt).contains(&(5.0, -0.1)));
    }

    #[test]
    fn a_slight_tower_is_given_longer_ribs_to_stand_on() {
        assert_eq!(least_depth(50.0), 20.0);
        assert_eq!(least_depth(175.0), 30.0);
        assert_eq!(least_depth(300.0), 40.0);
        // One change a layer makes a tower about 18 mm square, less than
        // the 20 mm its height asks for: its ribs are as long as a 20 mm
        // square's diagonal, and the extra on top.
        let settings = ribbed();
        let tower = plan(&[1, 1], &[0.2; 2], &settings).unwrap();
        let [left, _, right, _] = extent(&tower.layers[1].changes[0]);
        let span = right - left + 1.0;
        assert!(span < 20.0, "{span}");
        let [min_x, _, max_x, _] = extent([wall_of(&tower, 0)]);
        let shape = Shape {
            span,
            blocks: settings.block_depth(span, 0.2, false),
            height: 0.4,
            line: 0.5,
            wall: settings.wall,
        };
        let diagonal = span.hypot(shape.blocks);
        // The first of two layers has half of what a rib has over the
        // diagonal, shared between its ends. Along x that is the rib's
        // half length, and at most half its width for its end.
        let length = 20.0 * std::f64::consts::SQRT_2 + 8.0;
        let half_rib = diagonal / 2.0 + (length - diagonal) / 4.0;
        let reach = (max_x - min_x) / 2.0;
        let along = half_rib * span / diagonal;
        assert!(
            reach > along - 1.0 && reach < along + 4.0,
            "{reach} for {along}"
        );
    }

    #[test]
    fn no_tool_changes_or_no_tower_asked_for_means_no_tower() {
        let none = plan(&[0, 0, 0], &[0.2; 3], &on()).unwrap();
        assert!(none.footprint.is_none() && none.layers.iter().all(|l| l.finish.is_empty()));
        let off = TowerSettings {
            enabled: false,
            ..on()
        };
        let tower = plan(&[0, 1], &[0.2; 2], &off).unwrap();
        assert!(tower.footprint.is_none() && tower.layers[1].changes.is_empty());
    }

    #[test]
    fn settings_are_read_for_the_plate() {
        let read = |json: &str, plate: usize| {
            TowerSettings::from_settings(&Settings::from_json(json).unwrap(), plate).unwrap()
        };
        assert_eq!(read("{}", 0), TowerSettings::default());
        let json = r#"{"enable_prime_tower": "1", "prime_tower_width": "30", "prime_volume": "45",
            "prime_tower_brim_width": "5", "wipe_tower_x": ["13", "40", "13"],
            "wipe_tower_y": ["214.5", "200", "214.5"], "wipe_tower_extra_spacing": "120%",
            "wipe_tower_extra_flow": "100%", "wipe_tower_wall_type": "rib",
            "nozzle_diameter": ["0.4", "0.4"], "filament_multitool_ramming": ["0", "1"]}"#;
        let second = read(json, 1);
        assert!(second.enabled);
        assert_eq!(
            second.wall,
            Wall::Ribs {
                width: 8.0,
                extra: 0.0,
                rounded: true
            }
        );
        assert_eq!((second.x, second.y, second.width), (40.0, 200.0, 30.0));
        assert_eq!((second.extra_spacing, second.extra_flow), (1.2, 1.0));
        // A plate past the end of the list takes the last place given.
        assert_eq!(read(json, 9).x, 13.0);
        // Ramming is not done, and that is said; so is a wall of a kind
        // not made, and the gap a nozzle comes in by.
        assert_eq!(second.unsupported.len(), 1);
        let cone = read(
            r#"{"enable_prime_tower": "1", "wipe_tower_wall_type": "cone", "wipe_tower_wall_gap": "1"}"#,
            0,
        );
        assert_eq!(cone.wall, Wall::Box);
        assert_eq!(cone.unsupported.len(), 2);
        assert!(
            read(r#"{"wipe_tower_wall_type": "rib"}"#, 0)
                .unsupported
                .is_empty()
        );
    }

    #[test]
    fn a_tower_with_a_ribbed_wall_is_made_square() {
        // Five changes on a layer make a 30 mm tower 54 mm deep; as a
        // square of the same ground it is about 40 mm each way.
        let size = |settings: &TowerSettings| {
            let tower = plan(&[1, 5, 5], &[0.2; 3], settings).unwrap();
            let rows: Vec<&ExtrusionPath> = tower.layers[1].changes.iter().flatten().collect();
            let [min_x, min_y, max_x, max_y] = extent(rows);
            (max_x - min_x + 1.0, max_y - min_y + 0.6)
        };
        let (width, depth) = size(&on());
        assert!(
            (width - 30.0).abs() < 1e-6 && (depth - 54.0).abs() < 1e-6,
            "{width} x {depth}"
        );
        let (width, depth) = size(&ribbed());
        assert!((width - depth).abs() < 2.5, "{width} x {depth}");
        assert!((width - 40.5).abs() < 1e-6, "{width} x {depth}");
        // Every block still carries the plastic asked for.
        let tower = plan(&[1, 5, 5], &[0.2; 3], &ribbed()).unwrap();
        for block in &tower.layers[1].changes {
            let volume: f64 = block.iter().map(ExtrusionPath::volume_mm3).sum();
            assert!((45.0..45.0 * 1.2).contains(&volume), "{volume} mm³");
        }
    }
}
