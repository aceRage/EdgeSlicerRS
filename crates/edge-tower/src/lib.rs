//! The prime tower: a block beside the print where a nozzle just brought
//! into use lays down its first plastic, so that what oozed or dried while
//! it waited does not go into the part.
//!
//! A first working version after the reference's WipeTower2.cpp, for a
//! printer with a nozzle per filament, where nothing has to be flushed
//! between colours and every tool change primes the same `prime_volume`.
//! Kept from the reference: the line width, how deep a block a tool change
//! takes, that the tower stands on every layer up to the last tool change
//! and is never deeper above than below, a solid first layer, a brim, and
//! that a tower set to have a ribbed wall is made as wide as it is deep.
//!
//! Not done: the ribbed, filleted and cone walls themselves (the wall is
//! a plain box), the gap in the wall where a nozzle comes in, the slow ironing
//! pass that starts a block, turning the tower end for end on alternate
//! layers, ramming, and leaving out or thinning the layers no tool change
//! uses.

use std::f64::consts::PI;

use edge_config::{ConfigError, FloatOrPercent, Settings};
use edge_extrusion::{ExtrusionPath, Flow, FlowError, Role};
use edge_geom::Point2;

/// The tower's lines are this many nozzle diameters wide
/// (WipeTower2.hpp `Width_To_Nozzle_Ratio`).
const WIDTH_TO_NOZZLE: f64 = 1.25;
/// The reference plans a block as if two lines this many times wider than
/// the rest ran down its sides (`ramming_line_width_ratio`).
const RAMMING_WIDTH: f64 = 2.0;

/// The settings the tower depends on. Defaults are the reference's
/// (PrintConfig.cpp). Lengths are in mm.
#[derive(Clone, Debug, PartialEq)]
pub struct TowerSettings {
    pub enabled: bool,
    /// The front left corner of the tower's wall on the bed.
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
    /// Whether the tower is made as wide as it is deep, whatever width is
    /// set: the reference does so for its ribbed wall.
    pub squared: bool,
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
            squared: false,
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
                    "wipe_tower_wall_type",
                    "rectangle",
                    "the tower has a plain box for a wall",
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
            squared: settings.text("wipe_tower_wall_type") == Some("rib"),
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
    /// The wall, what fills the depth no tool change uses, and on the
    /// first layer the brim. Printed once, by whichever nozzle comes first.
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
    /// The ground the tower takes, brim included, as min x, min y, max x,
    /// max y in mm; none when there is no tower.
    pub footprint: Option<[f64; 4]>,
}

/// Plans the tower for a print whose layers are `heights` mm thick and
/// have `changes` tool changes each.
pub fn plan(
    changes: &[usize],
    heights: &[f64],
    settings: &TowerSettings,
) -> Result<Tower, FlowError> {
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
    let depth_at = |width: f64| {
        (0..=last)
            .map(|layer| {
                changes[layer] as f64 * settings.block_depth(width, heights[layer], layer == 0)
            })
            .fold(0.0, f64::max)
            + line
    };
    let mut width = settings.width.max(6.0 * line);
    // A tower with a ribbed wall takes the ground of the one set but as
    // a square, to the next whole line (WipeTower2.cpp `generate`).
    if settings.squared {
        width = ((depth_at(width) * width).sqrt() / line).ceil() * line;
    }
    let depth = depth_at(width);

    let at = |x: f64, y: f64| Point2::from_mm(settings.x + x, settings.y + y);
    for layer in 0..=last {
        let first = layer == 0;
        let flow = settings.flow(heights[layer])?;
        let path = |points: Vec<Point2>, role: Role| ExtrusionPath { points, role, flow };
        let mut finish = Vec::new();
        // The wall: one loop, its outside on the tower's edge.
        let half = line / 2.0;
        finish.push(path(
            vec![
                at(half, half),
                at(width - half, half),
                at(width - half, depth - half),
                at(half, depth - half),
                at(half, half),
            ],
            Role::PrimeTower,
        ));

        // Tool changes take their blocks from the front; each is rows
        // from side to side, the first of them laid slowly.
        let (left, right) = (1.5 * line, width - 1.5 * line);
        let step = line * settings.row_step(first);
        let block = settings.block_depth(width, heights[layer], first);
        let rows = (block / step).round().max(1.0) as usize;
        let mut front = line;
        for _ in 0..changes[layer] {
            let mut points = Vec::with_capacity(2 * rows);
            for row in 0..rows {
                let y = front + half + row as f64 * step;
                let (from, to) = if row % 2 == 0 {
                    (left, right)
                } else {
                    (right, left)
                };
                points.push(at(from, y));
                points.push(at(to, y));
            }
            let slow = points.drain(..2).collect();
            let mut block_paths = vec![path(slow, Role::PrimeLine)];
            if points.len() >= 2 {
                block_paths.push(path(points, Role::PrimeTower));
            }
            tower.layers[layer].changes.push(block_paths);
            front += block;
        }

        // The depth no tool change uses: solid on the bed, and above it
        // lines from front to back close enough together to carry the
        // layer over them.
        let back = depth - line;
        if back - front > line {
            let mut points = Vec::new();
            if first {
                let count = ((back - front) / line).floor().max(1.0) as usize;
                for row in 0..count {
                    let y = front + half + row as f64 * line;
                    let (from, to) = if row % 2 == 0 {
                        (left, right)
                    } else {
                        (right, left)
                    };
                    points.push(at(from, y));
                    points.push(at(to, y));
                }
            } else {
                let count = 1 + ((right - left) / settings.bridging).floor() as usize;
                let pitch = (right - left) / count as f64;
                for column in 0..=count {
                    let x = left + column as f64 * pitch;
                    let (from, to) = if column % 2 == 0 {
                        (front + half, back - half)
                    } else {
                        (back - half, front + half)
                    };
                    points.push(at(x, from));
                    points.push(at(x, to));
                }
            }
            finish.push(path(points, Role::PrimeTower));
        }

        // The brim holds the tower to the bed: loops out from the wall.
        if first {
            let spacing = flow.spacing;
            let loops = ((settings.brim_width + spacing / 2.0) / spacing).floor() as usize;
            for ring in (1..=loops).rev() {
                let out = ring as f64 * spacing - half;
                finish.push(path(
                    vec![
                        at(-out, -out),
                        at(width + out, -out),
                        at(width + out, depth + out),
                        at(-out, depth + out),
                        at(-out, -out),
                    ],
                    Role::PrimeTower,
                ));
            }
        }
        tower.layers[layer].finish = finish;
    }
    let brim = settings.brim_width;
    tower.footprint = Some([
        settings.x - brim,
        settings.y - brim,
        settings.x + width + brim,
        settings.y + depth + brim,
    ]);
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

    #[test]
    fn the_tower_stands_on_every_layer_up_to_the_last_change() {
        let settings = on();
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
        // The wall is the same on every layer: as deep as the busiest needs.
        let wall = |layer: usize| extent([&tower.layers[layer].finish[0]]);
        assert_eq!(wall(1), wall(3));
        let [min_x, min_y, max_x, max_y] = wall(3);
        assert!((min_x - 100.25).abs() < 1e-6 && (min_y - 50.25).abs() < 1e-6);
        assert!((max_x - 129.75).abs() < 1e-6);
        let depth = max_y - min_y + 0.5;
        let two_blocks = 2.0 * settings.block_depth(settings.width, 0.2, false) + 0.5;
        assert!(
            (depth - two_blocks).abs() < 1e-6,
            "{depth} for {two_blocks}"
        );
        // Everything a change lays down is inside the wall.
        for layer in &tower.layers {
            for block in &layer.changes {
                let [a, b, c, d] = extent(block);
                assert!(a > min_x && b > min_y && c < max_x && d < max_y);
            }
        }
        // Where a layer uses less depth, lines cross the rest no further
        // apart than the layer above can bridge.
        let across = &tower.layers[2].finish[1];
        let columns: Vec<f64> = across.points.iter().map(|p| unscaled(p.x)).collect();
        assert!(
            columns
                .windows(2)
                .all(|pair| (pair[1] - pair[0]).abs() <= 10.0)
        );
        assert!(columns.len() >= 8);
    }

    #[test]
    fn the_first_layer_is_solid_and_has_a_brim() {
        let settings = on();
        let tower = plan(&[0, 1], &[0.25, 0.2], &settings).unwrap();
        let first = &tower.layers[0].finish;
        // Wall, solid fill, and brim loops out to 5 mm.
        let spacing = settings.flow(0.25).unwrap().spacing;
        let loops = ((5.0 + spacing / 2.0) / spacing).floor() as usize;
        assert_eq!(first.len(), 2 + loops);
        let fill = &first[1];
        let rows: Vec<f64> = fill
            .points
            .iter()
            .step_by(2)
            .map(|p| unscaled(p.y))
            .collect();
        assert!(
            rows.windows(2)
                .all(|pair| (pair[1] - pair[0] - 0.5).abs() < 1e-6)
        );
        let [min_x, min_y, max_x, max_y] = extent(first);
        let reach = 100.0 - min_x;
        assert!(reach > 4.0 && reach <= 5.0, "brim reaches {reach}");
        // The ground it takes is known, for the skirt and the bed's edge.
        let [a, b, c, d] = tower.footprint.unwrap();
        assert!(a <= min_x && b <= min_y && c >= max_x && d >= max_y);
        assert_eq!((a, b), (95.0, 45.0));
        // No brim above the first layer.
        assert_eq!(tower.layers[1].finish.len(), 1);
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
        assert!(second.enabled && second.squared);
        assert_eq!((second.x, second.y, second.width), (40.0, 200.0, 30.0));
        assert_eq!((second.extra_spacing, second.extra_flow), (1.2, 1.0));
        // A plate past the end of the list takes the last place given.
        assert_eq!(read(json, 9).x, 13.0);
        // The ribbed wall and ramming are not done, and that is said.
        assert_eq!(second.unsupported.len(), 2);
        assert!(
            read(r#"{"wipe_tower_wall_type": "rib"}"#, 0)
                .unsupported
                .is_empty()
        );
    }

    #[test]
    fn a_tower_with_a_ribbed_wall_is_made_square() {
        // Five changes on a layer make a 30 mm tower 54.5 mm deep; as a
        // square of the same ground it is about 40 mm each way.
        let plain = on();
        let squared = TowerSettings {
            squared: true,
            ..on()
        };
        let size = |settings: &TowerSettings| {
            let [min_x, min_y, max_x, max_y] = plan(&[1, 5, 5], &[0.2; 3], settings)
                .unwrap()
                .footprint
                .unwrap();
            let brim = 2.0 * settings.brim_width;
            (max_x - min_x - brim, max_y - min_y - brim)
        };
        let (width, depth) = size(&plain);
        assert!(
            (width - 30.0).abs() < 0.6 && (depth - 54.5).abs() < 0.6,
            "{width} x {depth}"
        );
        let (width, depth) = size(&squared);
        assert!((width - depth).abs() < 2.5, "{width} x {depth}");
        assert!((width - 40.5).abs() < 1.0, "{width} x {depth}");
        // Every block still carries the plastic asked for.
        let tower = plan(&[1, 5, 5], &[0.2; 3], &squared).unwrap();
        for block in &tower.layers[1].changes {
            let volume: f64 = block.iter().map(ExtrusionPath::volume_mm3).sum();
            assert!((45.0..45.0 * 1.2).contains(&volume), "{volume} mm³");
        }
    }
}
