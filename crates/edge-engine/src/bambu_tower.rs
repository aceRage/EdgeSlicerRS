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
/// `nozzles` is the plate's filament grouping on a printer of two nozzles
/// (the H2D): which nozzle each filament prints from, so that a change
/// onto the other one rams the outgoing filament on the tower first.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(
    settings: &Settings,
    plate: usize,
    tower_settings: &TowerSettings,
    changes: &[Vec<Change>],
    heights: &[f64],
    tops: &[f64],
    start_tool: usize,
    nozzles: Option<&edge_gcode::NozzleMap>,
) -> Result<Laid, Failure> {
    let count = changes.len();
    let mut tower = Tower {
        layers: vec![TowerLayer::default(); count],
        footprint: None,
        envelope: None,
        mesh_bottom: None,
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
    // The grouping's nozzles, over the project's `filament_map` (Print.cpp
    // sets the map from the grouping before the tower).
    if let Some(map) = nozzles {
        let extruders: Vec<usize> = (0..t1.filaments.len())
            .map(|filament| map.extruder(filament))
            .collect();
        // Each filament at the size of nozzle it prints with, a rack
        // size it is pinned to included (`crate::rack`).
        let sizes: Vec<f32> = (crate::nozzle_process::filament_sizes(settings)?)
            .unwrap_or_default()
            .into_iter()
            .map(|size| size as f32)
            .collect();
        t1.map_nozzles(&extruders, &sizes);
    }
    // Print.cpp `_make_wipe_tower` (Bambu Studio's `is_need_ramming`): a
    // change between filaments on different nozzles of a Bambu Lab
    // printer rams the outgoing one first; a different extruder rams
    // `filament_change_length`, another hotend of one extruder `_nc`.
    let nozzle_change = |old: usize, new: usize| -> Option<type1::NozzleChange> {
        let map = nozzles?;
        if !t1.bambu {
            return None;
        }
        let nozzle = |filament: usize| map.nozzles.get(filament).copied().unwrap_or(-1);
        let (old_nozzle, new_nozzle) = (nozzle(old), nozzle(new));
        if old_nozzle < 0 || new_nozzle < 0 || old_nozzle == new_nozzle {
            return None;
        }
        Some(type1::NozzleChange {
            extruder_change: t1.hotend_as_extruder || map.extruder(old) != map.extruder(new),
            old_nozzle,
            new_nozzle,
            old_extruder: map.extruder(old),
        })
    };
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
                        nozzle_change: nozzle_change(old, change.filament),
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
    // Where a change G-code takes the head beside the tower on its way back
    // (GCode.cpp `bambu_wipe_avoid_pos_x` on `bambu_wipe_tower_plate_bbx`):
    // 3 mm right of the tower's first layer, brim included, else 3 mm left
    // of it, within X 100 to 250 (Bambu's numbers for its 350 mm beds),
    // else X 110.
    let [a, b, c, d] = planned.mesh_bottom()?;
    let (x, y) = (f64::from(t1.x), f64::from(t1.y));
    tower.mesh_bottom = Some([x + a, y + b, x + c, y + d]);
    let wipe_avoid_x = {
        let (width, _) = planned.footprint();
        // The brim as laid: whole loops and half a spacing
        // (`m_wipe_tower_brim_width_real`).
        let brim = {
            let first = planned
                .first_layer
                .min(planned.layers.len().saturating_sub(1));
            let height = planned.layers.get(first).map_or(0.2, |layer| layer.height);
            let spacing = planned.line_width - height * (1.0 - std::f32::consts::FRAC_PI_4);
            let loops = planned.brim_loops(first);
            if loops > 0 {
                f64::from(loops as f32 * spacing + spacing / 2.0)
            } else {
                0.0
            }
        };
        let right = origin[0] + f64::from(width) + brim + 3.0;
        let left = origin[0] - brim - 3.0;
        if right > 100.0 && right < 250.0 {
            right
        } else if left > 100.0 && left < 250.0 {
            left
        } else {
            110.0
        }
    };
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
        // The ramming before a change onto another nozzle, at its own
        // width.
        let ramming_flow = Flow::new(
            f64::from(planned.nozzle_change_width),
            height,
            f64::from(t1.nozzle_diameter),
        )?;
        for visit in layer {
            let mut lines = visit
                .nozzle_change
                .as_ref()
                .map(|ramming| {
                    // From where it starts, which its text does not say.
                    let from = format!(
                        "G1 X{} Y{}\n{}",
                        ramming.start_pos.0, ramming.start_pos.1, ramming.gcode
                    );
                    lines_of(&from, origin, ramming_flow)
                })
                .unwrap_or_default();
            lines.extend(lines_of(&visit.gcode, origin, flow));
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
                change_start_mm: [
                    f64::from(visit.tool_change_start_pos.0 + planned.origin.0),
                    f64::from(visit.tool_change_start_pos.1 + planned.origin.1),
                ],
                end: bed(visit.end_pos),
                wipe_path: visit.wipe_path.iter().map(|p| bed(*p)).collect(),
                finish_first: visit.is_finish_first,
                tool_change: visit.is_tool_change,
                reheats_after_cooling: visit.reheats_after_cooling,
                is_contact: visit.is_contact,
                wipe_avoid_x,
                nozzle_change: visit.nozzle_change.as_ref().map(|ramming| {
                    edge_gcode::TowerRamming {
                        gcode: ramming.gcode.clone(),
                        start: bed(ramming.start_pos),
                        end: bed(ramming.end_pos),
                        wipe_path: ramming.wipe_path.iter().map(|p| bed(*p)).collect(),
                    }
                }),
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
