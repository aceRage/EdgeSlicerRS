//! Bambu's prime tower (the reference's Type 1, WipeTower.cpp): planned and
//! laid by edge-tower's `type1`, and handed to the writer as the visits its
//! writer wrote ([`edge_gcode::TowerVisit`]), with the tower's lines for
//! everything else that looks at the tower (the toolhead clearance, the
//! preview, the tower's ground).

use edge_config::Settings;
use edge_extrusion::{ExtrusionPath, Flow, Role};
use edge_gcode::TowerVisit;
use edge_geom::Point2;
use edge_tower::{Block, Change, Tower, TowerLayer, TowerSettings, type1};

use crate::Failure;

/// The tower planned and laid: its layers, as the other towers give
/// them, and each layer's visits in the order they are made.
pub(crate) struct Laid {
    pub tower: Tower,
    pub visits: Vec<Vec<TowerVisit>>,
}

/// The lines a visit lays, in mm on the bed: every extruding move of its
/// text, an arc by its ends.
fn lines_of(gcode: &str, origin: [f64; 2], flow: Flow) -> Vec<ExtrusionPath> {
    let word = |line: &str, letter: char| -> Option<f64> {
        line.split_whitespace()
            .skip(1)
            .find_map(|w| w.strip_prefix(letter).and_then(|n| n.parse().ok()))
    };
    let mut paths: Vec<ExtrusionPath> = Vec::new();
    let mut at: Option<(f64, f64)> = None;
    let mut run: Vec<Point2> = Vec::new();
    let point = |(x, y): (f64, f64)| Point2::from_mm(x + origin[0], y + origin[1]);
    let close = |run: &mut Vec<Point2>, paths: &mut Vec<ExtrusionPath>| {
        if run.len() >= 2 {
            paths.push(ExtrusionPath {
                points: std::mem::take(run),
                role: Role::PrimeTower,
                flow,
                in_turn: false,
                part: 0,
            });
        }
        run.clear();
    };
    for line in gcode.lines() {
        let code = line.split(';').next().unwrap_or("").trim();
        if !(code.starts_with("G1 ") || code.starts_with("G2 ") || code.starts_with("G3 ")) {
            if line == "[change_filament_gcode]" {
                close(&mut run, &mut paths);
                at = None;
            }
            continue;
        }
        let (x, y) = (word(code, 'X'), word(code, 'Y'));
        if x.is_none() && y.is_none() {
            continue;
        }
        let here = at.unwrap_or((0.0, 0.0));
        let to = (x.unwrap_or(here.0), y.unwrap_or(here.1));
        if word(code, 'E').is_some_and(|e| e > 0.0) {
            if run.is_empty()
                && let Some(from) = at
            {
                run.push(point(from));
            }
            run.push(point(to));
        } else {
            close(&mut run, &mut paths);
        }
        at = Some(to);
    }
    close(&mut run, &mut paths);
    paths
}

/// Plans and lays Bambu's tower for the print's tool changes, `changes`
/// on each layer (the pipeline's), its layers `heights` thick and their
/// tops `tops`. `start_tool` is the filament the print starts with.
pub(crate) fn plan(
    settings: &Settings,
    plate: usize,
    tower_settings: &TowerSettings,
    changes: &[Vec<Change>],
    heights: &[f64],
    tops: &[f64],
    start_tool: usize,
) -> Result<Laid, Failure> {
    let count = changes.len();
    let mut tower = Tower {
        layers: vec![TowerLayer::default(); count],
        footprint: None,
        envelope: None,
    };
    let mut visits = vec![Vec::new(); count];
    let Some(mut last) = changes.iter().rposition(|layer| !layer.is_empty()) else {
        return Ok(Laid { tower, visits });
    };
    let mut t1 = type1::Settings::from_settings(settings, plate)?;
    // Laid as the tower laid them before this module, each said in a note
    // by `TowerSettings::from_settings`: the layers with no tool change
    // one by one (`wipe_tower_sparse_layers_combination`), and the tower
    // interface options left out.
    t1.combine_sparse = false;
    t1.edge_interface = false;
    // A smooth timelapse photographs the tower on every layer.
    if t1.smooth_timelapse {
        last = count - 1;
    }
    // Where the pipeline put the tower (moved onto the bed when its
    // place no longer fits it).
    t1.x = tower_settings.x as f32;
    t1.y = tower_settings.y as f32;
    let input: Vec<type1::LayerIn> = (0..=last)
        .map(|index| type1::LayerIn {
            z: tops[index] as f32,
            height: heights[index] as f32,
            changes: changes[index]
                .iter()
                .filter_map(|change| {
                    change.old.map(|old| type1::ChangeIn {
                        old,
                        new: change.filament,
                        purge_volume: 0.0,
                    })
                })
                .collect(),
        })
        .collect();
    let planned = type1::plan(&t1, &input, start_tool)?;
    let laid = type1::lay(&planned, &t1)?;
    let origin = [f64::from(planned.origin.0), f64::from(planned.origin.1)];
    let bed =
        |p: (f32, f32)| Point2::from_mm(f64::from(p.0) + origin[0], f64::from(p.1) + origin[1]);
    let mut stacked = 0.0;
    for (index, layer) in laid.iter().enumerate() {
        let out = &mut tower.layers[index];
        if planned.laid(index) || !planned.skip_sparse {
            stacked += heights[index];
        }
        if planned.laid(index) {
            out.z = planned.skip_sparse.then_some(stacked);
            out.first_speed = index == planned.first_layer;
        }
        let height = heights[index];
        let flow = Flow::new(
            f64::from(planned.line_width),
            height,
            f64::from(t1.nozzle_diameter),
        )?;
        for visit in layer {
            let lines = lines_of(&visit.gcode, origin, flow);
            if visit.is_tool_change {
                out.changes.push(Block {
                    lines,
                    ..Block::default()
                });
            } else {
                out.finish.extend(lines);
            }
            visits[index].push(TowerVisit {
                gcode: visit.gcode.clone(),
                origin,
                start: bed(visit.start_pos),
                change_start: bed(visit.tool_change_start_pos),
                end: bed(visit.end_pos),
                wipe_path: visit.wipe_path.iter().map(|p| bed(*p)).collect(),
                finish_first: visit.is_finish_first,
                tool_change: visit.is_tool_change,
                reheats_after_cooling: visit.reheats_after_cooling,
            });
        }
    }
    // The ground it takes: its first layer's lines, out to their edges.
    let half = f64::from(planned.line_width) / 2.0;
    let first = &tower.layers[planned.first_layer.min(count - 1)];
    tower.footprint = first
        .finish
        .iter()
        .chain(first.changes.iter().flat_map(|block| &block.lines))
        .flat_map(|path| &path.points)
        .map(|p| (edge_geom::unscaled(p.x), edge_geom::unscaled(p.y)))
        .fold(None, |ground: Option<[f64; 4]>, (x, y)| {
            Some(match ground {
                None => [x, y, x, y],
                Some([a, b, c, d]) => [a.min(x), b.min(y), c.max(x), d.max(y)],
            })
        })
        .map(|[a, b, c, d]| [a - half, b - half, c + half, d + half]);
    Ok(Laid { tower, visits })
}
