//! The prime tower: a block beside the print where a nozzle just brought
//! into use lays down its first plastic, so that what oozed or dried while
//! it waited does not go into the part.
//!
//! A first working version after the reference's WipeTower2.cpp, for a
//! printer with a nozzle per filament, where nothing has to be flushed
//! between colours and every tool change primes the same `prime_volume`.
//! Kept from the reference: the line width, how deep a block a tool change
//! takes, that the tower stands on every layer up to the last tool change
//! and is never deeper above than below (a layer is as deep as it and
//! those above it need, and sits in the middle of the tower), a solid first layer, the lines
//! that carry the next layer over depth no tool change uses, the ribbed
//! wall (a tower with one is made as wide as it is deep, has a rib along
//! each diagonal that runs out past the corners, longest on the bed and
//! gone at the top, and stands where the ribs' ends put it), and the brim,
//! which follows the wall and steps in a loop a layer above the first.
//!
//! Lengths here are the reference's own: the wall's middle runs from the
//! tower's corner, so the tower's outside is half a line beyond it.
//!
//! A filament set to be rammed (`filament_multitool_ramming`) is, by the
//! nozzle being put away: a short wide line at the front of the block,
//! which the new nozzle's first line then runs on from. The wall is left
//! open where the new nozzle comes in (`wipe_tower_wall_gap`).
//!
//! Not done: the cone wall, wiping the nozzle flat on the tower as it
//! comes in, starting each block at a different place, turning the tower
//! end for end on alternate layers, and leaving out or thinning the
//! layers no tool change uses.

use std::f64::consts::PI;

use edge_clip::{ClipError, Join};
use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionPath, Flow, FlowError, Role};
use edge_geom::{ExPolygon, Point2, Polygon, scaled, unscaled};

/// The tower's lines are this many nozzle diameters wide
/// (WipeTower2.hpp `Width_To_Nozzle_Ratio`).
const WIDTH_TO_NOZZLE: f64 = 1.25;
/// The wall is left open this many line widths either side of where a
/// nozzle comes in (WipeTower2.cpp `generate_support_rib_wall`).
const GAP_HALF: f64 = 2.5;

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

/// How a filament is rammed before its nozzle is put away: pushed out
/// fast, so that what is left in the nozzle comes away clean.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramming {
    /// How much, in mm³ (`filament_multitool_ramming_volume`).
    pub volume: f64,
    /// How fast, in mm³ a second (`filament_multitool_ramming_flow`).
    pub flow: f64,
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
    /// Whether the wall is left open where a nozzle comes into the tower
    /// (`wipe_tower_wall_gap`).
    pub wall_gap: bool,
    /// A ramming line is this many tower lines wide, and a block is
    /// planned as if one ran down each of its sides
    /// (`ramming_line_width_ratio`).
    pub ram_width: f64,
    /// How each filament is rammed, in the settings' order; none for one
    /// that is not.
    pub ramming: Vec<Option<Ramming>>,
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
            wall_gap: true,
            ram_width: 2.0,
            ramming: Vec::new(),
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
                    "the wall is left open, but the nozzle is not wiped flat on the tower as it comes in",
                ),
                (
                    "wipe_tower_stagger_toolchange_start",
                    "0",
                    "every tool change starts at the same place in its block",
                ),
                (
                    "enable_change_pressure_when_wiping",
                    "0",
                    "pressure advance is left as it is while a filament is rammed",
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
        }
        // A filament is rammed when it is set to be and has something to
        // ram with (WipeTower2.cpp `set_extruder`).
        let list = |key: &str| -> Result<Vec<Option<f64>>, ConfigError> {
            Ok(settings.floats(key)?.unwrap_or_default())
        };
        let (volumes, flows) = (
            list("filament_multitool_ramming_volume")?,
            list("filament_multitool_ramming_flow")?,
        );
        let ramming = settings
            .texts("filament_multitool_ramming")
            .map(|rammed| {
                rammed
                    .iter()
                    .enumerate()
                    .map(|(filament, rammed)| {
                        let of = |values: &[Option<f64>]| {
                            values
                                .get(filament)
                                .or(values.last())
                                .copied()
                                .flatten()
                                .unwrap_or(10.0)
                        };
                        let (volume, flow) = (of(&volumes), of(&flows));
                        (rammed == "1" && volume > 0.0 && flow > 0.0)
                            .then_some(Ramming { volume, flow })
                    })
                    .collect()
            })
            .unwrap_or_default();
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
            wall_gap: settings
                .flag("wipe_tower_wall_gap")?
                .unwrap_or(defaults.wall_gap),
            ram_width: number("ramming_line_width_ratio", defaults.ram_width)?.max(1.0),
            ramming,
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

    /// What one tool change takes of a tower `span` mm wide on a layer
    /// `height` mm thick, when the filament put away is rammed as `ram`
    /// says (WipeTower2.cpp `set_toolchange`, `get_wipe_depth`): rows
    /// enough, across a width a little less than the tower's, to lay
    /// down `prime_volume`, after the rows the ram takes.
    fn planned(&self, span: f64, height: f64, first_layer: bool, ram: Option<Ramming>) -> Planned {
        let line = WIDTH_TO_NOZZLE * self.nozzle_diameter;
        let wide = line * self.ram_width;
        let per_mm = |width: f64| height * (width - height * (1.0 - PI / 4.0));
        let across = (span - line - 2.0 * wide).max(line);
        let (ram_length, ramming, first_wipe) = match ram {
            Some(ram) => {
                let length = ram.volume / per_mm(wide);
                let rows = (length / across).ceil().max(1.0);
                // The new nozzle's first line finishes the ram's last row.
                let over = length - (length / across).floor() * across;
                (length, rows * wide * self.extra_spacing, across - over)
            }
            None => (0.0, 0.0, 0.0),
        };
        let already = first_wipe * per_mm(line * self.extra_flow);
        let length = ((self.prime_volume - already) / per_mm(line) / self.extra_flow).max(0.0);
        let rows = (length / across).floor() + 1.0;
        Planned {
            ram_length,
            ramming,
            first_wipe,
            wiping: rows * line * self.row_step(first_layer),
        }
    }

    /// How much of the tower's depth a tool change with no ramming takes.
    #[cfg(test)]
    fn block_depth(&self, span: f64, height: f64, first_layer: bool) -> f64 {
        self.planned(span, height, first_layer, None).depth()
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

/// The room one tool change is given.
#[derive(Clone, Copy, Debug)]
struct Planned {
    /// How long the ram is, in mm; nothing when there is none.
    ram_length: f64,
    /// The depth the ram's rows take.
    ramming: f64,
    /// How much of the ram's last row is left for the new nozzle.
    first_wipe: f64,
    /// The depth the new nozzle's rows take.
    wiping: f64,
}

impl Planned {
    fn depth(&self) -> f64 {
        self.ramming + self.wiping
    }
}

/// What the nozzle being put away lays down first.
#[derive(Clone, Debug)]
pub struct Rammed {
    pub lines: Vec<ExtrusionPath>,
    /// How fast it is laid, in mm a second: the filament's ramming flow.
    pub speed: f64,
    /// Where that nozzle is then taken, clear of what it has just laid.
    pub park: Point2,
}

/// What one tool change has on the tower.
#[derive(Clone, Debug, Default)]
pub struct Block {
    /// The ram, when the filament put away is rammed.
    pub ram: Option<Rammed>,
    /// The way in for the new nozzle: from outside the wall, through the
    /// gap left for it. Empty when the wall has no gap.
    pub enter: Vec<Point2>,
    /// What the new nozzle lays down.
    pub lines: Vec<ExtrusionPath>,
}

/// What the tower has on one layer.
#[derive(Clone, Debug, Default)]
pub struct TowerLayer {
    /// What fills the depth no tool change uses, the wall, and the brim.
    /// Printed once, by whichever nozzle comes first.
    pub finish: Vec<ExtrusionPath>,
    /// What each tool change on the layer has, in order.
    pub changes: Vec<Block>,
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

/// Where a line across at `y` from the wall's left side (or its right)
/// first meets the outline going outward: how far along the outline
/// that is from its first point, and the place itself.
fn crossing(outline: &[(f64, f64)], left: bool, from: f64, y: f64) -> Option<(f64, (f64, f64))> {
    let mut along = 0.0;
    let mut best: Option<(f64, f64, (f64, f64))> = None;
    for (index, a) in outline.iter().enumerate() {
        let b = outline[(index + 1) % outline.len()];
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        if (a.1 - y) * (b.1 - y) <= 0.0 && a.1 != b.1 {
            let share = (y - a.1) / (b.1 - a.1);
            let x = a.0 + share * (b.0 - a.0);
            let out = if left { from - x } else { x - from };
            if out >= -1e-6 && best.is_none_or(|(least, ..)| out < least) {
                best = Some((out, along + share * length, (x, y)));
            }
        }
        along += length;
    }
    best.map(|(_, along, place)| (along, place))
}

/// A closed outline with a stretch `half` mm either side of each of
/// `gaps` left out, each given as how far along the outline it is from
/// its first point. What is left, as open lines.
fn opened(outline: &[(f64, f64)], gaps: &[f64], half: f64) -> Vec<Vec<(f64, f64)>> {
    let count = outline.len();
    let mut marks = Vec::with_capacity(count + 1);
    let mut total = 0.0;
    for (index, a) in outline.iter().enumerate() {
        marks.push(total);
        let b = outline[(index + 1) % count];
        total += (b.0 - a.0).hypot(b.1 - a.1);
    }
    if gaps.is_empty() || total <= 0.0 {
        return vec![outline.iter().chain(outline.first()).copied().collect()];
    }
    let place = |along: f64| -> (f64, f64) {
        let along = along.rem_euclid(total);
        let index = marks.partition_point(|mark| *mark <= along).max(1) - 1;
        let (a, b) = (outline[index], outline[(index + 1) % count]);
        let length = (b.0 - a.0).hypot(b.1 - a.1);
        let share = if length > 0.0 {
            (along - marks[index]) / length
        } else {
            0.0
        };
        (a.0 + share * (b.0 - a.0), a.1 + share * (b.1 - a.1))
    };
    // Measured from the far end of the first gap, so none lies across
    // the start.
    let origin = gaps[0] + half;
    let mut out: Vec<(f64, f64)> = Vec::new();
    for gap in gaps {
        let from = (gap - half - origin).rem_euclid(total);
        let to = from + 2.0 * half;
        if to > total {
            out.push((from, total));
            out.push((0.0, to - total));
        } else {
            out.push((from, to));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut pieces = Vec::new();
    let mut at = 0.0;
    for (from, to) in out.into_iter().chain([(total, total)]) {
        if from - at > 1e-6 {
            let (start, end) = (origin + at, origin + from);
            let mut piece = vec![place(start)];
            let base = start.rem_euclid(total);
            for turn in 0..2 * count {
                let mark = marks[turn % count] + (turn / count) as f64 * total;
                if mark > base + 1e-9 && mark < base + (end - start) - 1e-9 {
                    piece.push(outline[turn % count]);
                }
            }
            piece.push(place(end));
            pieces.push(piece);
        }
        at = at.max(to);
    }
    pieces
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
    /// How far back a layer `deep` mm deep starts, so that it sits in
    /// the middle of the tower (`m_y_shift`).
    fn lift(&self, deep: f64) -> f64 {
        if deep < self.blocks - 1e-9 {
            (self.blocks - deep) / 2.0
        } else {
            0.0
        }
    }

    /// The middle of the wall's line on the layer whose top is `z` mm
    /// up (WipeTower2.cpp `support_rib_wall_polygon`).
    /// `deep` is what that layer's blocks and those above it take, which
    /// may be less than the tower's: its box is then that much shallower
    /// and sits in the middle, while the ribs stay where they are.
    fn wall(&self, z: f64, deep: f64) -> Result<Vec<(f64, f64)>, ClipError> {
        // The tower's depth is what its blocks take and the wall's line.
        let (span, depth) = (self.span, self.blocks + self.line);
        let front = self.lift(deep);
        let back = front + deep + self.line;
        let plain = [(0.0, front), (span, front), (span, back), (0.0, back)];
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
        let diagonal = span.hypot(depth);
        let mut length = 0.0f64;
        let least = least_depth(self.height);
        if depth + 1e-4 < least {
            length = least * std::f64::consts::SQRT_2;
        }
        let length = (length.max(diagonal) + extra).max(diagonal);
        // Past each corner by half of what is over, less the higher up.
        let past = (length - diagonal) / 2.0 * (self.height - z).abs() / self.height;
        let half = width.min(depth.min(span) / 2.0) / 2.0;
        let mut shapes = Vec::with_capacity(3);
        for (from, to) in [((0.0, 0.0), (span, depth)), ((span, 0.0), (0.0, depth))] {
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

/// Plans the tower for a print whose layers are `heights` mm thick.
/// `changes` has, for each layer, its tool changes in order: for each,
/// how the filament put away is rammed, or none when it is not.
pub fn plan(
    changes: &[Vec<Option<Ramming>>],
    heights: &[f64],
    settings: &TowerSettings,
) -> Result<Tower, TowerError> {
    let count = changes.len();
    let mut tower = Tower {
        layers: vec![TowerLayer::default(); count],
        footprint: None,
    };
    let Some(last) = changes.iter().rposition(|changes| !changes.is_empty()) else {
        return Ok(tower);
    };
    if !settings.enabled {
        return Ok(tower);
    }
    let line = WIDTH_TO_NOZZLE * settings.nozzle_diameter;
    let wide = line * settings.ram_width;
    // What each layer's tool changes take of a tower `span` wide.
    let used_at = |span: f64, layer: usize| -> f64 {
        changes[layer]
            .iter()
            .map(|ram| {
                settings
                    .planned(span, heights[layer], layer == 0, *ram)
                    .depth()
            })
            .sum()
    };
    // A layer is as deep as its tool changes need, and no shallower than
    // any layer above it, so nothing is laid over air.
    let blocks_at = |span: f64| {
        (0..=last)
            .map(|layer| used_at(span, layer))
            .fold(0.0, f64::max)
    };
    let mut span = settings.width.max(6.0 * line);
    // A tower with a ribbed wall takes the ground of the one set but as
    // a square, to the next whole line (WipeTower2.cpp `generate`).
    if matches!(settings.wall, Wall::Ribs { .. }) {
        span = (((blocks_at(span) + line) * span).sqrt() / line).ceil() * line;
    }
    let shape = Shape {
        span,
        blocks: blocks_at(span),
        height: heights[..=last].iter().sum(),
        line,
        wall: settings.wall,
    };
    // A layer is as deep as its own tool changes and those of the layers
    // above it need, and no deeper (`plan_tower`).
    let mut deep = vec![0.0; last + 1];
    let mut most: f64 = 0.0;
    for layer in (0..=last).rev() {
        most = most.max(used_at(span, layer));
        deep[layer] = most;
    }
    // The tower stands where the wall of its first layer, ribs and all,
    // has its corner on the place set (`set_rib_offset`).
    let first_wall = shape.wall(heights[0], deep[0])?;
    let corner = first_wall
        .iter()
        .fold((f64::MAX, f64::MAX), |(x, y), p| (x.min(p.0), y.min(p.1)));
    let place =
        |x: f64, y: f64| Point2::from_mm(settings.x - corner.0 + x, settings.y - corner.1 + y);

    let mut top = 0.0;
    for layer in 0..=last {
        let first = layer == 0;
        let height = heights[layer];
        top += height;
        let flow = settings.flow(height)?;
        let path = |points: Vec<Point2>| ExtrusionPath {
            points,
            role: Role::PrimeTower,
            flow,
            in_turn: false,
        };
        let per_mm = |width: f64| height * (width - height * (1.0 - PI / 4.0));
        // What the layer lays is measured from its own front.
        let blocks = deep[layer];
        let lift = shape.lift(blocks);
        let at = |x: f64, y: f64| place(x, y + lift);

        // Tool changes take their blocks from the front
        // (`toolchange_Unload`, `toolchange_Wipe`).
        let step = line * settings.row_step(first);
        let row = line * settings.extra_flow;
        let mut used = 0.0;
        // Where each new nozzle comes in: which side, and how far back.
        let mut ways_in: Vec<(bool, f64)> = Vec::with_capacity(changes[layer].len());
        for ram in &changes[layer] {
            let planned = settings.planned(span, height, first, *ram);
            let mut block = Block::default();
            let (mut x, mut y, mut rightward) = (1.5 * line, used + step - row / 2.0, true);
            if let Some(ram) = ram {
                // The ram: a wide line from the left, turning at the far
                // side for as long as it lasts.
                let pitch = wide * settings.extra_spacing;
                let (left, right) = (1.5 * line, span - 1.5 * line);
                y = line / 2.0 + used + pitch / 2.0;
                let mut room = right - left;
                let mut lines = Vec::new();
                // Where the ram would end over the open part of the layer
                // below, a line is laid up to it first for it to rest on.
                let below = if layer > 1 {
                    Some(used_at(span, layer - 1) + line)
                } else {
                    None
                };
                let lead = planned.first_wipe - wide;
                if below.is_some_and(|open| used + planned.ramming > open + 0.5 * line)
                    && lead > 0.0
                {
                    lines.push(path(vec![at(x, y), at(x + lead, y)]));
                    x += lead;
                    room -= lead;
                }
                let rammed = Flow::new(wide, height, settings.nozzle_diameter)?;
                let mut left_over = planned.ram_length;
                let mut points = vec![at(x, y)];
                while left_over > 1e-4 {
                    let stretch = left_over.min(room);
                    x += if rightward { stretch } else { -stretch };
                    points.push(at(x, y));
                    left_over -= stretch;
                    room -= stretch;
                    if room < 1e-3 {
                        lines.push(ExtrusionPath {
                            points: std::mem::take(&mut points),
                            role: Role::PrimeTower,
                            flow: rammed,
                            in_turn: false,
                        });
                        y += pitch;
                        rightward = !rightward;
                        room = right - left;
                        points.push(at(x, y));
                    }
                }
                if points.len() >= 2 {
                    lines.push(ExtrusionPath {
                        points,
                        role: Role::PrimeTower,
                        flow: rammed,
                        in_turn: false,
                    });
                }
                // The new nozzle's rows carry on in the far half of the
                // ram's last, and the old one waits a line beyond that.
                y += (wide - line) / 2.0;
                block.ram = Some(Rammed {
                    lines,
                    speed: ram.flow / per_mm(wide),
                    park: at(x, y + line),
                });
            }
            // Its way in is through the wall on the side the ram ended
            // nearer, level with where the ram's rows end
            // (`get_all_wall_skip_points`).
            let from_left = {
                let (left, right) = (line / 2.0 + wide, span - line / 2.0 - wide);
                let reach = right - left;
                let end = if ram.is_none() || reach <= 1e-3 {
                    left
                } else {
                    let turns = (planned.ram_length / reach).floor();
                    let over = planned.ram_length - turns * reach;
                    if (turns as usize).is_multiple_of(2) {
                        left + over
                    } else {
                        right - over
                    }
                };
                end < span / 2.0
            };
            ways_in.push((from_left, used + planned.ramming + line / 2.0));

            // The new nozzle's rows, from side to side: until the block
            // is full where nothing was rammed, and otherwise until the
            // plastic asked for is down.
            let (left, right) = (line / 2.0, span - line / 2.0);
            let whole = ram.is_none();
            let mut owed = settings.prime_volume / per_mm(line) / settings.extra_flow;
            let full = used + planned.depth();
            if (if rightward { right - x } else { x - left }) < 2.5 * row {
                x = if rightward { right - row } else { left + row };
                y += step;
                rightward = !rightward;
            }
            let mut points = vec![at(x, y)];
            loop {
                let before = x;
                x = if rightward {
                    right - line / 2.0
                } else {
                    left + line / 2.0
                };
                points.push(at(x, y));
                if y + 1e-4 > full - 0.5 * row {
                    break;
                }
                owed -= (before - x).abs();
                if !whole && owed < 1e-3 {
                    break;
                }
                y += step;
                points.push(at(x, y));
                rightward = !rightward;
            }
            // The first of them is laid slowly.
            block.lines.push(ExtrusionPath {
                points: points[..2].to_vec(),
                role: Role::PrimeLine,
                flow,
                in_turn: false,
            });
            if points.len() > 2 {
                block.lines.push(path(points[1..].to_vec()));
            }
            tower.layers[layer].changes.push(block);
            used += planned.depth();
        }

        // The depth no tool change uses (`finish_layer`): a frame, and
        // inside it lines solid on the bed, or above it from front to
        // back close enough together to carry the layer over them.
        let mut finish = Vec::new();
        let (left, right) = (line, span - line);
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

        // The wall, its line's middle on the outline, left open where a
        // nozzle comes in.
        let wall = shape.wall(top, blocks)?;
        let closed = |outline: &[(f64, f64)]| -> Vec<Point2> {
            outline
                .iter()
                .chain(outline.first())
                .map(|(x, y)| place(*x, *y))
                .collect()
        };
        let mut gaps = Vec::new();
        if settings.wall_gap {
            for (block, (from_left, y)) in tower.layers[layer].changes.iter_mut().zip(&ways_in) {
                let side = if *from_left { 0.0 } else { span };
                let Some((along, (x, y))) = crossing(&wall, *from_left, side, *y + lift) else {
                    continue;
                };
                gaps.push(along);
                let out = if *from_left { -1.0 } else { 1.0 };
                block.enter = vec![
                    place(x + out * 3.0 * line, y),
                    place(x - out * line / 2.0, y),
                ];
            }
        }
        if gaps.is_empty() {
            finish.push(path(closed(&wall)));
        } else {
            for piece in opened(&wall, &gaps, GAP_HALF * line) {
                finish.push(path(piece.iter().map(|(x, y)| place(*x, *y)).collect()));
            }
        }

        // The brim holds the tower to the bed: loops out from the wall,
        // all of them on the first layer and one fewer each layer above
        // it, from no wider than the chamfer allows.
        let spacing = flow.spacing;
        let mut loops = ((settings.brim_width + spacing / 2.0) / spacing) as usize;
        if !first {
            // The brim does not go on up a tower that has got shallower.
            let as_deep = (blocks - deep[0]).abs() < 1e-9;
            loops = if settings.brim_chamfer && as_deep {
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

    /// So many tool changes a layer, none of them with a ram.
    fn counts(changes: &[usize]) -> Vec<Vec<Option<Ramming>>> {
        changes.iter().map(|count| vec![None; *count]).collect()
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
        let tower = plan(&counts(&[0, 1, 0]), &[0.2; 3], &settings).unwrap();
        let block = &tower.layers[1].changes[0].lines;
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
        let mut rows: Vec<f64> = block
            .iter()
            .flat_map(|path| &path.points)
            .map(|p| unscaled(p.y))
            .collect();
        rows.dedup();
        assert!(
            rows.windows(2)
                .all(|pair| (pair[1] - pair[0] - 0.6).abs() < 1e-6),
            "{rows:?}"
        );
        // The block is full: its last row is a row's step short of the
        // next block's first.
        let depth = settings.block_depth(settings.width, 0.2, false);
        assert!((rows.len() as f64 * 0.6 - depth).abs() < 1e-6);
        // From one row to the next the nozzle keeps laying.
        assert!(block[1].points.len() > 2 * (rows.len() - 1));
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
            wall_gap: false,
            ..on()
        };
        // Changes on layers 1 and 3 of six; two on layer 3.
        let tower = plan(&counts(&[0, 1, 0, 2, 0, 0]), &[0.2; 6], &settings).unwrap();
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
                let [a, b, c, d] = extent(&block.lines);
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
        let tower = plan(&counts(&[0, 1]), &[0.25, 0.2], &settings).unwrap();
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
            let tower = plan(&counts(&changes), &heights, settings).unwrap();
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
        let tower = plan(&counts(&changes), &heights, &settings).unwrap();
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
        let tower = plan(&counts(&[1, 5, 5, 5, 5]), &[0.2; 5], &settings).unwrap();
        // The rows give the tower's own box: a line inside each wall.
        let [left, front, right, _] = extent(&tower.layers[1].changes[0].lines);
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
        let tower = plan(&counts(&[1, 5, 5, 5, 5]), &[0.2; 5], &sharp).unwrap();
        let wall = wall_of(&tower, 0);
        let [left, ..] = extent(&tower.layers[1].changes[0].lines);
        let [min_x, ..] = extent([wall]);
        assert!(left - 0.5 - min_x > past);
        // A box with two bars across it: each bar's end is two corners,
        // and it leaves the box at two more.
        assert_eq!(wall.points.len(), 16 + 1);
    }

    #[test]
    fn the_wall_is_left_open_where_each_nozzle_comes_in() {
        let settings = TowerSettings {
            brim_chamfer: false,
            ..on()
        };
        let whole = TowerSettings {
            wall_gap: false,
            ..settings.clone()
        };
        let changes = counts(&[1, 2, 2]);
        let heights = [0.2; 3];
        let open = plan(&changes, &heights, &settings).unwrap();
        let shut = plan(&changes, &heights, &whole).unwrap();
        // Shut, the wall is one closed line and nothing says how to come in.
        let wall = wall_of(&shut, 1);
        assert_eq!(wall.points.first(), wall.points.last());
        assert!(
            shut.layers[1]
                .changes
                .iter()
                .all(|block| block.enter.is_empty())
        );
        // Open, it is short by 2.5 mm for each of the layer's two changes:
        // two and a half lines either side of where the nozzle comes in.
        // (The layer's changes take all its depth, so nothing but wall
        // is laid here.)
        let pieces: Vec<&ExtrusionPath> = open.layers[1]
            .finish
            .iter()
            .filter(|path| path.points.first() != path.points.last())
            .collect();
        let laid: f64 = pieces.iter().map(|piece| piece.length_mm()).sum();
        assert!((wall.length_mm() - laid - 2.0 * 2.5).abs() < 1e-3, "{laid}");
        // Each way in starts outside the wall, level with the block's
        // first row and a quarter of a line below it, and ends inside.
        let depth = settings.block_depth(settings.width, 0.2, false);
        for (index, block) in open.layers[1].changes.iter().enumerate() {
            let [out, into] = block.enter[..] else {
                panic!("two points on the way in")
            };
            assert!(unscaled(out.x) < 100.0 && unscaled(into.x) > 100.0);
            let level = 50.0 + index as f64 * depth + 0.25;
            assert!((unscaled(out.y) - level).abs() < 1e-6 && out.y == into.y);
            let first = block.lines[0].points[0];
            assert!((unscaled(first.y) - level - 0.1).abs() < 1e-6);
            // No piece of wall lies across it.
            for piece in &pieces {
                for pair in piece.points.windows(2) {
                    let on_left = unscaled(pair[0].x) < 100.001 && unscaled(pair[1].x) < 100.001;
                    let (low, high) = (pair[0].y.min(pair[1].y), pair[0].y.max(pair[1].y));
                    assert!(!(on_left && low < out.y && out.y < high));
                }
            }
        }
    }

    #[test]
    fn an_outline_is_opened_either_side_of_each_gap() {
        let square = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)];
        let length = |piece: &[(f64, f64)]| {
            piece
                .windows(2)
                .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
                .sum::<f64>()
        };
        // Going across from the left side at half height, the outline is
        // met on its last side, 35 mm round.
        let (along, place) = crossing(&square, true, 0.0, 5.0).unwrap();
        assert!((along - 35.0).abs() < 1e-9 && place == (0.0, 5.0));
        let (along, place) = crossing(&square, false, 10.0, 2.0).unwrap();
        assert!((along - 12.0).abs() < 1e-9 && place == (10.0, 2.0));
        // One gap leaves one line, from its far end round to its near.
        let pieces = opened(&square, &[35.0], 1.0);
        assert_eq!(pieces.len(), 1);
        assert_eq!(
            (pieces[0][0], *pieces[0].last().unwrap()),
            ((0.0, 4.0), (0.0, 6.0))
        );
        assert!((length(&pieces[0]) - 38.0).abs() < 1e-9);
        assert_eq!(pieces[0].len(), 6);
        // Two leave two; a gap across the outline's first point is whole.
        let pieces = opened(&square, &[0.5, 20.0], 1.0);
        assert_eq!(pieces.len(), 2);
        assert!((pieces.iter().map(|p| length(p)).sum::<f64>() - 36.0).abs() < 1e-9);
        assert!(pieces.iter().flatten().all(|p| *p != (0.0, 0.0)));
        // Gaps that run into each other leave one line.
        let pieces = opened(&square, &[5.0, 6.0], 1.0);
        assert_eq!(pieces.len(), 1);
        assert!((length(&pieces[0]) - 37.0).abs() < 1e-9);
        // With no gap the outline is closed.
        let pieces = opened(&square, &[], 1.0);
        assert_eq!(pieces[0].len(), 5);
    }

    #[test]
    fn a_rammed_filament_is_pushed_out_before_the_new_nozzle_primes() {
        let settings = TowerSettings {
            wall_gap: false,
            brim_chamfer: false,
            ..on()
        };
        let ram = Ramming {
            volume: 0.5,
            flow: 20.0,
        };
        let plain = plan(&counts(&[0, 1]), &[0.2; 2], &settings).unwrap();
        let rammed = plan(&[vec![], vec![Some(ram)]], &[0.2; 2], &settings).unwrap();
        assert!(plain.layers[1].changes[0].ram.is_none());
        let block = &rammed.layers[1].changes[0];
        let laid = block.ram.as_ref().unwrap();
        // One line twice as wide as the tower's, as long as holds the
        // half cubic millimetre, at the speed that makes 20 mm³ a second.
        assert_eq!(laid.lines.len(), 1);
        let line = &laid.lines[0];
        assert_eq!(line.flow.width, 1.0);
        assert!((line.volume_mm3() - 0.5).abs() < 1e-6);
        assert!((line.length_mm() - 2.612).abs() < 1e-3);
        assert!((laid.speed - 104.48).abs() < 0.01, "{}", laid.speed);
        // It starts a line and a half in from the wall and its row is
        // 1.2 mm deep; the tool then waits three quarters of a line on.
        let (from, to) = (line.points[0], line.points[1]);
        assert!(
            (unscaled(from.x) - 100.75).abs() < 1e-6 && (unscaled(from.y) - 50.85).abs() < 1e-6
        );
        assert_eq!((laid.park.x, unscaled(laid.park.y)), (to.x, 51.6));
        // The new nozzle carries on from where the ram stopped, in the
        // far half of its row, to the far wall.
        let first = &block.lines[0];
        assert_eq!(first.role, Role::PrimeLine);
        assert_eq!(first.points[0].x, to.x);
        assert!((unscaled(first.points[0].y) - 51.1).abs() < 1e-6);
        assert!((unscaled(first.points[1].x) - 129.5).abs() < 1e-6);
        // It lays what was asked for and stops; the block is planned
        // with the ram's row and one row to spare.
        let volume: f64 = block.lines.iter().map(ExtrusionPath::volume_mm3).sum();
        assert!((45.0..45.0 * 1.1).contains(&volume), "{volume} mm³");
        let (with, without) = (
            settings.planned(30.0, 0.2, false, Some(ram)),
            settings.planned(30.0, 0.2, false, None),
        );
        assert!((with.ramming - 1.2).abs() < 1e-9 && without.ramming == 0.0);
        assert!(
            (with.depth() - (1.2 + 17.0 * 0.6)).abs() < 1e-9,
            "{}",
            with.depth()
        );
        assert!((without.depth() - 18.0 * 0.6).abs() < 1e-9);
        // The wall is as deep as that.
        let [_, min_y, _, max_y] = extent([wall_of(&rammed, 1)]);
        assert!((max_y - min_y - with.depth() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_long_ram_turns_at_the_far_side() {
        let settings = TowerSettings {
            wall_gap: false,
            ..on()
        };
        // 12 mm³ is 62.7 mm of ramming line; a row holds 28.5 mm.
        let ram = Ramming {
            volume: 12.0,
            flow: 10.0,
        };
        let tower = plan(&[vec![], vec![Some(ram)]], &[0.2; 2], &settings).unwrap();
        let block = &tower.layers[1].changes[0];
        let laid = block.ram.as_ref().unwrap();
        assert_eq!(laid.lines.len(), 3);
        let length: f64 = laid.lines.iter().map(ExtrusionPath::length_mm).sum();
        assert!((length - 12.0 / (0.2 * (1.0 - 0.2 * (1.0 - PI / 4.0)))).abs() < 1e-6);
        let rows: Vec<f64> = laid.lines.iter().map(|l| unscaled(l.points[0].y)).collect();
        assert!((rows[1] - rows[0] - 1.2).abs() < 1e-6 && (rows[2] - rows[1] - 1.2).abs() < 1e-6);
        // Out, back, and part of the way out again.
        let ends: Vec<f64> = laid.lines.iter().map(|l| unscaled(l.points[1].x)).collect();
        assert!((ends[0] - 129.25).abs() < 1e-6 && (ends[1] - 100.75).abs() < 1e-6);
        assert!(ends[2] > 100.75 && ends[2] < 129.25);
        // The room planned for it is three rows, and the new nozzle's
        // lines start above them.
        let planned = settings.planned(30.0, 0.2, false, Some(ram));
        assert!((planned.ramming - 3.0 * 1.2).abs() < 1e-9);
        assert!(unscaled(block.lines[0].points[0].y) > rows[2]);
    }

    #[test]
    fn a_layer_is_no_deeper_than_it_and_those_above_need() {
        // Two changes a layer, then one, then one more layer of one.
        let settings = TowerSettings {
            wall_gap: false,
            ..on()
        };
        let tower = plan(&counts(&[2, 2, 1, 1]), &[0.2; 4], &settings).unwrap();
        let one = settings.block_depth(settings.width, 0.2, false);
        let wall = |layer: usize| {
            // The wall is the one closed line that is not a brim loop:
            // the innermost.
            let closed: Vec<&ExtrusionPath> = tower.layers[layer]
                .finish
                .iter()
                .filter(|path| path.points.first() == path.points.last())
                .collect();
            closed
                .iter()
                .map(|path| extent([*path]))
                .min_by(|a, b| (a[3] - a[1]).total_cmp(&(b[3] - b[1])))
                .unwrap()
        };
        // Below, the box is two blocks deep and starts at the place set.
        let [_, front, _, back] = wall(1);
        assert!((front - 50.0).abs() < 1e-6 && (back - front - 2.0 * one - 0.5).abs() < 1e-6);
        // Above, it is one block deep and sits in the middle of that.
        let [left, front, right, back] = wall(2);
        assert!((back - front - one - 0.5).abs() < 1e-6, "{}", back - front);
        assert!((front - 50.0 - one / 2.0).abs() < 1e-6, "{front}");
        assert!((left - 100.0).abs() < 1e-6 && (right - 130.0).abs() < 1e-6);
        assert_eq!(wall(2), wall(3));
        // Its one block is inside it, and nothing is laid across depth
        // the layer does not have.
        let [_, low, _, high] = extent(&tower.layers[2].changes[0].lines);
        assert!(low > front && high < back);
        assert_eq!(tower.layers[2].finish.len(), 1);
        // A layer below that has a change to spare gets its frame and
        // lines across, as before.
        let spare = plan(&counts(&[1, 2]), &[0.2; 2], &settings).unwrap();
        assert!(spare.layers[0].finish.len() > 3);
        // The brim steps in up the tower only while the tower is as deep
        // as it was on the bed.
        let brimmed = |layer: usize| {
            tower.layers[layer]
                .finish
                .iter()
                .filter(|path| path.points.first() == path.points.last())
                .count()
                - 1
        };
        assert!(brimmed(1) > 0);
        assert_eq!(brimmed(2), 0);
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
        let tower = plan(&counts(&[1, 1]), &[0.2; 2], &settings).unwrap();
        let [left, _, right, _] = extent(&tower.layers[1].changes[0].lines);
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
        let none = plan(&counts(&[0, 0, 0]), &[0.2; 3], &on()).unwrap();
        assert!(none.footprint.is_none() && none.layers.iter().all(|l| l.finish.is_empty()));
        let off = TowerSettings {
            enabled: false,
            ..on()
        };
        let tower = plan(&counts(&[0, 1]), &[0.2; 2], &off).unwrap();
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
        // The second filament is rammed, with the reference's own
        // amounts where none are given.
        assert_eq!(
            second.ramming,
            [
                None,
                Some(Ramming {
                    volume: 10.0,
                    flow: 10.0
                })
            ]
        );
        assert_eq!(second.ram_width, 2.0);
        assert!(second.wall_gap && second.unsupported.is_empty());
        let rammed = read(
            r#"{"filament_multitool_ramming": ["1", "1"], "filament_multitool_ramming_volume": ["0.5", "0"],
                "filament_multitool_ramming_flow": ["20", "20"], "ramming_line_width_ratio": "4",
                "wipe_tower_wall_gap": "0"}"#,
            0,
        );
        assert_eq!(
            rammed.ramming,
            [
                Some(Ramming {
                    volume: 0.5,
                    flow: 20.0
                }),
                None
            ]
        );
        assert!(!rammed.wall_gap && rammed.ram_width == 4.0);
        // A wall of a kind not made is said, and so is what goes with
        // the gap and is not done.
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
            let tower = plan(&counts(&[1, 5, 5]), &[0.2; 3], settings).unwrap();
            let rows: Vec<&ExtrusionPath> = tower.layers[1]
                .changes
                .iter()
                .flat_map(|block| &block.lines)
                .collect();
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
        let tower = plan(&counts(&[1, 5, 5]), &[0.2; 3], &ribbed()).unwrap();
        for block in &tower.layers[1].changes {
            let volume: f64 = block.lines.iter().map(ExtrusionPath::volume_mm3).sum();
            assert!((45.0..45.0 * 1.2).contains(&volume), "{volume} mm³");
        }
    }
}
