//! Which nozzle each filament prints from on a printer of two nozzles of
//! different kinds (the Bambu Lab H2D), worked out from a plate's settings
//! and the filaments each layer prints (ToolOrdering.cpp
//! `reorder_extruders_for_minimum_flush_volume`,
//! `get_recommended_filament_maps`, `build_filament_group_context`;
//! the grouping itself is `edge-group`'s).
//!
//! Refused here, where the reference would group differently from what is
//! ported: nozzle racks (H2C, H3C), a part or the prime tower where only
//! one nozzle reaches or above a nozzle's height, and a flush matrix of a
//! length the grouping cannot read (see `edge_group::prepare_flush_matrices`).

use std::collections::{BTreeMap, BTreeSet};

use edge_config::{Settings, reference_defaults};
use edge_group::{
    ChangeTimes, FlushMatrix, Input, Machine, Mode, Usage, VolumeType, nozzle_list,
    other_layers_sequences, prepare_flush_matrices,
};

use crate::Failure;

/// What the grouping worked out for a plate, and what it went by.
#[derive(Clone, Debug)]
pub struct Grouped {
    pub machine: Machine,
    pub input: Input,
    /// `flush_volumes_matrix`, `flush_multiplier` and, where
    /// `prime_volume_mode` is "Fast", `flush_multiplier_fast`, as read, so
    /// the matrices can be made again the reference's way.
    pub flush_values: Vec<f64>,
    pub flush_multiplier: f64,
    pub flush_fast: Option<Vec<f64>>,
    /// The answer: none where there was nothing to group.
    pub grouping: Option<edge_group::Grouping>,
    /// Each layer's filaments, from 0, in the order the layer prints them;
    /// with `preload_all_filaments`, the first layer loads every other
    /// filament ahead of its own (ToolOrdering.cpp
    /// `apply_preload_all_filaments`), which the slice itself does not do
    /// yet (it says so in its notes).
    pub orders: Vec<Vec<u32>>,
}

/// The project's settings over the reference's defaults, for the keys the
/// grouping reads.
fn merged(settings: &Settings) -> Settings {
    let mut merged = reference_defaults().clone();
    merged.overlay(settings);
    merged
}

fn numbers(settings: &Settings, key: &str) -> Result<Vec<f64>, Failure> {
    Ok((settings.floats(key)?.unwrap_or_default().into_iter())
        .map(|value| value.unwrap_or(0.0))
        .collect())
}

/// A nozzle diameter as the reference writes it for the grouping
/// (PrintConfig.cpp `format_diameter_to_str`): two decimals, trailing
/// zeros and point dropped.
fn diameter_text(diameter: f64) -> String {
    let text = format!("{diameter:.2}");
    let text = text.trim_end_matches('0');
    text.trim_end_matches('.').to_owned()
}

fn volume_type(name: &str) -> Result<VolumeType, Failure> {
    VolumeType::from_name(name.trim()).ok_or_else(|| {
        format!("nozzle_volume_type \"{name}\": not a nozzle kind the grouping knows").into()
    })
}

/// The printer, as the grouping sees it: one nozzle to each extruder
/// (ToolOrdering.cpp `build_nozzle_groups` and MultiNozzleUtils.cpp
/// `build_nozzle_list`), the main extruder, and how many filaments each
/// extruder takes before a grouping is the worse for it.
fn machine(settings: &Settings) -> Result<Machine, Failure> {
    let diameters = numbers(settings, "nozzle_diameter")?;
    let extruders = diameters.len();
    let rack = |key: &str| {
        (settings.texts(key)).is_some_and(|texts| texts.iter().any(|t| !t.trim().is_empty()))
    };
    let counts = numbers(settings, "extruder_max_nozzle_count")?;
    if settings.flag("handle_hotend_as_extruder")? == Some(true)
        || counts.iter().any(|count| *count > 1.0)
        || rack("extruder_rack_nozzle_diameter")
        || rack("extruder_rack_nozzle_override")
    {
        return Err(
            "a printer with a nozzle rack (H2C, H3C): grouping filaments onto the rack's \
                    nozzles is not done yet; no G-code was written"
                .into(),
        );
    }
    let kinds = settings.texts("nozzle_volume_type").unwrap_or_default();
    let stats = settings.texts("extruder_nozzle_stats").unwrap_or_default();
    let mut groups = Vec::new();
    for (extruder, diameter) in diameters.iter().enumerate() {
        // A missing entry is a Standard nozzle (`nozzle_volume_type_at`).
        let kind = match kinds.get(extruder) {
            Some(name) => volume_type(name)?,
            None => VolumeType::STANDARD,
        };
        let text = diameter_text(*diameter);
        // "Kind#count|Kind#count" (PrintConfig.cpp `get_extruder_nozzle_stats`).
        let mut stat: BTreeMap<VolumeType, i32> = BTreeMap::new();
        for entry in stats.get(extruder).map_or("", String::as_str).split('|') {
            let mut parts = entry.split('#');
            if let (Some(name), Some(count)) = (parts.next(), parts.next()) {
                let kind = VolumeType::from_name(name).unwrap_or(VolumeType::STANDARD);
                *stat.entry(kind).or_default() += count.trim().parse::<i32>().unwrap_or(0);
            }
        }
        let e = extruder as i32;
        if stat.is_empty() {
            groups.push((text, kind, e, 1));
        } else if kind == VolumeType::HYBRID {
            for (kind, count) in stat {
                groups.push((text.clone(), kind, e, count));
            }
        } else {
            groups.push((text, kind, e, stat.get(&kind).copied().unwrap_or(0)));
        }
    }
    let nozzles = nozzle_list(&groups);
    if nozzles.len() != extruders
        || (nozzles.iter().enumerate()).any(|(index, nozzle)| nozzle.extruder_id != index as i32)
    {
        return Err(format!(
            "extruder_nozzle_stats {stats:?}: an extruder of other than one nozzle; grouping \
             filaments onto such nozzles is not done yet; no G-code was written"
        )
        .into());
    }
    let filaments = settings.texts("filament_colour").map_or(0, <[String]>::len) as i32;
    // The spools each extruder is fed from come from the printer, and the
    // command line has none to ask: one each (FilamentGroupUtils.cpp
    // `calc_max_group_size`), every filament with a filament switcher.
    let mut max_group_size = vec![1; 2];
    if settings.flag("has_filament_switcher")? == Some(true) {
        for size in &mut max_group_size {
            *size = (*size).max(filaments);
        }
    }
    Ok(Machine {
        extruders,
        nozzles,
        master_extruder: settings.integer("master_extruder_id")?.unwrap_or(1) as i32 - 1,
        max_group_size,
        // Only a Bowden extruder would rather print support, and the
        // reference reads `extruder_type` as one Direct Drive entry
        // whatever the project says (a legacy key, dropped on load).
        prefer_non_model: vec![false; extruders],
    })
}

/// Where the parts and the prime tower lie for the nozzles: the
/// filaments each extruder cannot print (Print.cpp, before the tower,
/// `detect_extruder_geometric_unprintables` and the tower's reach). Every
/// set is empty when each part and the tower lie where both nozzles reach
/// and below both nozzles' heights; anything else is refused, as working
/// out which filaments print there is not done.
fn geometric(
    settings: &Settings,
    objects: &[(Vec<[f64; 2]>, f64)],
    tower: Option<[f64; 4]>,
) -> Result<Vec<BTreeSet<i32>>, Failure> {
    let extruders = numbers(settings, "nozzle_diameter")?.len();
    let empty = vec![BTreeSet::new(); extruders];
    let point = |text: &str| -> Option<[f64; 2]> {
        let (x, y) = text.trim().split_once('x')?;
        Some([x.trim().parse().ok()?, y.trim().parse().ok()?])
    };
    let polygon = |text: &str| -> Vec<[f64; 2]> { text.split(',').filter_map(point).collect() };
    let bed: Vec<[f64; 2]> = (settings.texts("printable_area").unwrap_or_default().iter())
        .filter_map(|p| point(p))
        .collect();
    let mut areas: Vec<Vec<[f64; 2]>> = (settings
        .texts("extruder_printable_area")
        .unwrap_or_default()
        .iter())
    .flat_map(|text| text.split('#'))
    .map(polygon)
    .filter(|area| !area.is_empty())
    .collect();
    // One entry of two rectangles is two areas
    // (ExtruderAreas.cpp `split_merged_extruder_areas`).
    if areas.len() == 1 && areas[0].len() == 8 {
        let second = areas[0].split_off(4);
        areas.push(second);
    }
    if areas.len() < 2 || bed.len() < 3 {
        return Ok(empty);
    }
    // A missing or degenerate area is the whole bed.
    let areas: Vec<Vec<[f64; 2]>> = (areas.into_iter())
        .map(|area| if area.len() >= 3 { area } else { bed.clone() })
        .collect();
    let heights = numbers(settings, "extruder_printable_height")?;
    let limit = |e: usize| {
        heights
            .get(e)
            .copied()
            .filter(|h| h.is_finite() && *h > 0.0)
            .unwrap_or(0.0)
    };
    let limited = areas.len() >= 2 && (1..areas.len()).any(|e| (limit(e) - limit(0)).abs() > 1e-6);
    for (hull, top) in objects {
        for (e, area) in areas.iter().enumerate() {
            if !hull.iter().all(|p| inside(area, *p, 1e-6)) {
                return Err("a part lies where only one of the two nozzles reaches: which filaments \
                            that nozzle cannot then print is not worked out yet; no G-code was written"
                    .into());
            }
            if limited && limit(e) > 0.0 && *top > limit(e) + 0.01 {
                return Err("a part is taller than one of the two nozzles can print: which filaments \
                            that nozzle cannot then print is not worked out yet; no G-code was written"
                    .into());
            }
        }
    }
    if let Some([x0, y0, x1, y1]) = tower {
        let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        if !(areas.iter()).all(|area| corners.iter().all(|c| inside(area, *c, 0.5))) {
            return Err(
                "the prime tower stands where only one of the two nozzles reaches, or \
                        near it: grouping the filaments away from that nozzle is not worked out \
                        yet; no G-code was written"
                    .into(),
            );
        }
    }
    Ok(empty)
}

/// Whether `p` lies in the convex polygon `area` (either winding) or
/// within `tolerance` of it. A polygon that is not convex holds nothing.
fn inside(area: &[[f64; 2]], p: [f64; 2], tolerance: f64) -> bool {
    let n = area.len();
    let twice_area: f64 = (0..n)
        .map(|i| {
            let (a, b) = (area[i], area[(i + 1) % n]);
            a[0] * b[1] - b[0] * a[1]
        })
        .sum();
    if n < 3 || twice_area == 0.0 {
        return false;
    }
    let turning = twice_area.signum();
    for i in 0..n {
        let (a, b, c) = (area[i], area[(i + 1) % n], area[(i + 2) % n]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        // Every corner turns the way the whole does.
        let turn = dx * (c[1] - b[1]) - dy * (c[0] - b[0]);
        if turn * turning < 0.0 {
            return false;
        }
        let length = dx.hypot(dy);
        if length == 0.0 {
            continue;
        }
        // How far p lies on the inner side of the edge.
        let depth = (dx * (p[1] - a[1]) - dy * (p[0] - a[0])) / length * turning;
        if depth < -tolerance {
            return false;
        }
    }
    true
}

/// The grouping for a plate on a printer the reference groups filaments
/// onto two nozzles for. `layers` is each layer's filaments, from 0, in
/// the order the layer lists them before it is put in order; `orders` the
/// order each layer prints them in; `objects` each part's outline on the
/// bed (its points) and height, `tower` the prime tower's box, if any.
pub(crate) fn group(
    settings: &Settings,
    layers: Vec<Vec<u32>>,
    orders: Vec<Vec<u32>>,
    objects: &[(Vec<[f64; 2]>, f64)],
    tower: Option<[f64; 4]>,
) -> Result<Grouped, Failure> {
    let settings = &merged(settings);
    let machine = machine(settings)?;
    let filament_count = settings.texts("filament_colour").map_or(0, <[String]>::len);
    let mode_name = settings
        .text("filament_map_mode")
        .unwrap_or("Auto For Flush");
    let mode = Mode::from_name(mode_name).ok_or_else(|| {
        format!("filament_map_mode \"{mode_name}\": not a grouping mode the reference knows")
    })?;
    let flush_values = numbers(settings, "flush_volumes_matrix")?;
    let flush_multiplier = numbers(settings, "flush_multiplier")?
        .first()
        .copied()
        .unwrap_or(1.0);
    let flush_fast = (settings.text("prime_volume_mode") == Some("Fast"))
        .then(|| numbers(settings, "flush_multiplier_fast"))
        .transpose()?;
    let flush: Vec<FlushMatrix> = prepare_flush_matrices(
        &flush_values,
        machine.extruders,
        filament_count,
        flush_multiplier,
        flush_fast.as_deref(),
        false,
    )?;
    let whole = |key: &str| -> Result<Vec<i64>, Failure> {
        Ok(numbers(settings, key)?
            .into_iter()
            .map(|n| n as i64)
            .collect())
    };
    let sequences = other_layers_sequences(
        settings
            .integer("other_layers_print_sequence_nums")?
            .unwrap_or(0),
        &whole("other_layers_print_sequence")?,
    );
    let float = |key: &str| -> Result<f64, Failure> { Ok(settings.float(key)?.unwrap_or(0.0)) };
    let input = Input {
        mode,
        filament_count,
        layer_filaments: layers,
        flush,
        // What each filament prints only counts on an extruder that would
        // rather print support, and there is none (see `machine`).
        usage: vec![Usage::ModelOnly; filament_count],
        diameters: numbers(settings, "filament_diameter")?
            .into_iter()
            .map(|d| d as f32)
            .collect(),
        geometric: geometric(settings, objects, tower)?,
        sequences,
        times: ChangeTimes::from_machine(
            float("machine_load_filament_time")?,
            float("machine_unload_filament_time")?,
        ),
        manual_map: whole("filament_map")?
            .into_iter()
            .map(|e| e as i32)
            .collect(),
    };
    let grouping = edge_group::group_filaments(&machine, &input)?;
    let mut orders = orders;
    if let Some(grouping) = &grouping
        && settings.flag("preload_all_filaments")? == Some(true)
        && settings.flag("enable_prime_tower")? == Some(true)
        && let Some(first) = orders.iter_mut().find(|order| !order.is_empty())
    {
        let extruders = grouping.result.extruder_map(true);
        *first = edge_group::preload_first_layer_tool_order(first, &grouping.result.used, |f| {
            extruders.get(f as usize).copied().unwrap_or(0)
        });
    }
    Ok(Grouped {
        machine,
        input,
        flush_values,
        flush_multiplier,
        flush_fast,
        grouping,
        orders,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diameters_are_written_short() {
        assert_eq!(diameter_text(0.4), "0.4");
        assert_eq!(diameter_text(0.25), "0.25");
        assert_eq!(diameter_text(1.0), "1");
    }

    #[test]
    fn points_inside_a_rectangle_either_way_round() {
        let square = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let back: Vec<[f64; 2]> = square.iter().rev().copied().collect();
        for area in [&square[..], &back[..]] {
            assert!(inside(area, [5.0, 5.0], 0.0));
            assert!(inside(area, [10.0, 5.0], 0.0));
            assert!(!inside(area, [10.2, 5.0], 0.0));
            assert!(inside(area, [10.2, 5.0], 0.5));
        }
    }

    fn h2d() -> Settings {
        Settings::from_json(
            r##"{"nozzle_diameter": ["0.4", "0.4"], "printable_area": ["0x0", "350x0", "350x320", "0x320"],
                "extruder_printable_area": ["0x0,325x0,325x320,0x320", "25x0,350x0,350x320,25x320"],
                "extruder_printable_height": ["320", "325"], "master_extruder_id": "2",
                "filament_colour": ["#FFFFFF", "#202020"],
                "flush_volumes_matrix": ["0", "199", "639", "0"], "flush_multiplier": "1",
                "machine_load_filament_time": "26", "machine_unload_filament_time": "26"}"##,
        )
        .unwrap()
    }

    #[test]
    fn parts_where_one_nozzle_reaches_are_refused() {
        let settings = h2d();
        let middle = vec![(vec![[100.0, 100.0], [150.0, 120.0]], 20.0)];
        assert!(
            geometric(&settings, &middle, Some([40.0, 220.0, 100.0, 230.0]))
                .unwrap()
                .iter()
                .all(BTreeSet::is_empty)
        );
        let edge = vec![(vec![[10.0, 100.0], [150.0, 120.0]], 20.0)];
        assert!(geometric(&settings, &edge, None).is_err());
        let tall = vec![(vec![[100.0, 100.0]], 322.0)];
        assert!(geometric(&settings, &tall, None).is_err());
        assert!(geometric(&settings, &middle, Some([330.0, 220.0, 340.0, 230.0])).is_err());
    }

    #[test]
    fn the_tower_stands_where_both_nozzles_reach() {
        // PartPlate.cpp `prime_tower_placement_box`: the H2D's left nozzle
        // reaches 0-325 mm, its right 25-350.
        let bed = [0.0, 0.0, 350.0, 320.0];
        assert_eq!(
            edge_tower::placement_box(&h2d(), bed).unwrap(),
            [25.0, 0.0, 325.0, 320.0]
        );
        let mut one = h2d();
        one.insert(
            "nozzle_diameter",
            edge_config::Value::Many(vec!["0.4".into()]),
        );
        assert_eq!(edge_tower::placement_box(&one, bed).unwrap(), bed);
    }

    #[test]
    fn two_filaments_on_an_h2d_go_one_to_each_nozzle() {
        let grouped = group(&h2d(), vec![vec![0, 1]; 3], Vec::new(), &[], None).unwrap();
        // Either way round costs the same, and the first tried wins:
        // filament 1 on the right nozzle. The reference, reading the one
        // matrix as half a matrix to each nozzle, gives [1, 2]
        // (work/bambu/h2d_two.3mf).
        assert_eq!(grouped.grouping.unwrap().filament_map, [2, 1]);
        let mut rack = h2d();
        rack.insert(
            "extruder_max_nozzle_count",
            edge_config::Value::Many(vec!["1".into(), "6".into()]),
        );
        let said = group(&rack, vec![vec![0, 1]], Vec::new(), &[], None)
            .unwrap_err()
            .to_string();
        assert!(said.contains("rack"), "{said}");
    }
}
