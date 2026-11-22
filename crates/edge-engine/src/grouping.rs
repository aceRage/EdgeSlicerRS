//! Which nozzle each filament prints from on a printer of two nozzles of
//! different kinds (the Bambu Lab H2D) or with a nozzle rack (the H2C:
//! one nozzle on the left, six on the right), worked out from a plate's settings
//! and the filaments each layer prints (ToolOrdering.cpp
//! `reorder_extruders_for_minimum_flush_volume`,
//! `get_recommended_filament_maps`, `build_filament_group_context`;
//! the grouping itself is `edge-group`'s).
//!
//! Refused here, where the reference would group differently from what is
//! ported: rack hotends taken as extruders (the H3C), a rack of mixed
//! nozzle kinds, what `crate::rack` refuses of racks set by hand and pins,
//! a part or the prime tower where only
//! one nozzle reaches or above a nozzle's height, and a flush matrix of a
//! length the grouping cannot read (see `edge_group::prepare_flush_matrices`).

use std::collections::{BTreeMap, BTreeSet};

use edge_config::{Settings, reference_defaults};
use edge_group::{
    ChangeTimes, FlushMatrix, Input, Machine, Mode, NozzleInfo, Usage, VolumeType, nozzle_list,
    other_layers_sequences, prepare_flush_matrices,
};

use edge_gcode::NozzleMap;

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
    // Two nozzles of different sizes print each with a process of its own
    // size (`crate::nozzle_process`), which needs the filament map before
    // the walls are laid: only a map set by hand comes this far
    // (`nozzle_process::fill` refuses the others).
    let counts = numbers(settings, "extruder_max_nozzle_count")?;
    let count_of = |extruder: usize| counts.get(extruder).map_or(1, |c| (*c as i32).max(1));
    // Rack hotends taken one by one as extruders (Bambu Studio 2.9.1
    // `handle_hotend_as_extruder`; PrintConfig.cpp `hotend_as_extruder`
    // also says so of one extruder with a rack): the H3C.
    if settings.flag("handle_hotend_as_extruder")? == Some(true)
        || (extruders == 1 && count_of(0) > 1)
    {
        return Err(
            "a printer whose rack hotends count as extruders (H3C): grouping filaments onto \
             them is not done yet; no G-code was written"
                .into(),
        );
    }
    // A nozzle rack (the H2C's right extruder, `extruder_max_nozzle_count`
    // above one): its nozzles as the project, the printer or the machine
    // names them (`crate::rack`), each a place a filament can be grouped
    // onto.
    let multi_nozzle = counts.iter().any(|count| *count > 1.0);
    let racks = crate::rack::racks(settings);
    let kinds = settings.texts("nozzle_volume_type").unwrap_or_default();
    let stats = settings.texts("extruder_nozzle_stats").unwrap_or_default();
    let total_slots: i32 = (0..extruders).map(count_of).sum();
    let slot_kinds = settings
        .texts("default_nozzle_volume_type")
        .unwrap_or_default();
    let mut groups = Vec::new();
    for (extruder, diameter) in diameters.iter().enumerate() {
        // A missing entry is a Standard nozzle (`nozzle_volume_type_at`).
        let kind = match kinds.get(extruder) {
            Some(name) => volume_type(name)?,
            None => VolumeType::STANDARD,
        };
        let text = diameter_text(*diameter);
        let e = extruder as i32;
        let count = count_of(extruder);
        if count > 1 {
            if kind == VolumeType::HYBRID {
                return Err(format!(
                    "a nozzle rack of mixed kinds (extruder {}, nozzle_volume_type Hybrid): \
                     grouping filaments onto it is not done yet; no G-code was written",
                    extruder + 1
                )
                .into());
            }
            // One kind for each slot when the list is as long as all slots
            // (the H3C's TPU slot); the H2C's has one to an extruder.
            if slot_kinds.len() == total_slots as usize && total_slots as usize > extruders {
                let first: i32 = (0..extruder).map(count_of).sum();
                for name in &slot_kinds[first as usize..(first + count) as usize] {
                    let slot = volume_type(name)?;
                    if slot != kind && slot != VolumeType::STANDARD && slot != VolumeType::HYBRID {
                        return Err(
                            "a nozzle rack of mixed kinds (default_nozzle_volume_type): \
                                    grouping filaments onto it is not done yet; no G-code was \
                                    written"
                                .into(),
                        );
                    }
                }
            }
        }
        // ToolSlots.cpp `rack_nozzle_groups`: an extruder with a rack has
        // one group of each size and kind of its rack's nozzles, in rack
        // order (the kinds are all the extruder's, `rack::validate`).
        if let Some(rack) = racks.get(extruder).filter(|rack| !rack.is_empty()) {
            let mut sizes: Vec<(String, i32)> = Vec::new();
            for nozzle in rack {
                let size = diameter_text(nozzle.diameter);
                match sizes.iter_mut().find(|(own, _)| *own == size) {
                    Some((_, count)) => *count += 1,
                    None => sizes.push((size, 1)),
                }
            }
            for (size, count) in sizes {
                groups.push((size, kind, e, count));
            }
            continue;
        }
        // "Kind#count|Kind#count" (PrintConfig.cpp `get_extruder_nozzle_stats`).
        let mut stat: BTreeMap<VolumeType, i32> = BTreeMap::new();
        for entry in stats.get(extruder).map_or("", String::as_str).split('|') {
            let mut parts = entry.split('#');
            if let (Some(name), Some(count)) = (parts.next(), parts.next()) {
                let kind = VolumeType::from_name(name).unwrap_or(VolumeType::STANDARD);
                *stat.entry(kind).or_default() += count.trim().parse::<i32>().unwrap_or(0);
            }
        }
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
    // A printer with a rack numbers its nozzles by physical slot
    // (ToolSlots.cpp `grouping_nozzle_list`): an extruder's rack in rack
    // order, any other extruder's nozzles sorted (`build_nozzle_list`),
    // extruder 0 first; the numbers reach the G-code (`filament_nozzle_map`)
    // and the printer checks the hotend in that slot. Without a rack the
    // sorted list, as before.
    let nozzles = if racks.iter().all(Vec::is_empty) {
        nozzle_list(&groups)
    } else {
        let mut list = Vec::new();
        for extruder in 0..extruders {
            let own: Vec<NozzleInfo> = match racks.get(extruder).filter(|rack| !rack.is_empty()) {
                Some(rack) => (rack.iter())
                    .map(|nozzle| NozzleInfo {
                        diameter: diameter_text(nozzle.diameter),
                        volume_type: nozzle.kind,
                        extruder_id: extruder as i32,
                        group_id: 0,
                    })
                    .collect(),
                None => nozzle_list(
                    &(groups.iter())
                        .filter(|group| group.2 == extruder as i32)
                        .cloned()
                        .collect::<Vec<_>>(),
                ),
            };
            for mut nozzle in own {
                nozzle.group_id = list.len() as i32;
                list.push(nozzle);
            }
        }
        list
    };
    let one_each = (0..extruders).all(|extruder| {
        let on = nozzles
            .iter()
            .filter(|n| n.extruder_id == extruder as i32)
            .count();
        on == 1
            || racks
                .get(extruder)
                .is_some_and(|rack| on == rack.len() && on > 0)
    });
    if nozzles.is_empty() || !one_each {
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
        multi_nozzle,
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
/// `by_plate_map`: the plate's own `filament_map` is taken whatever
/// `filament_map_mode` says, as the reference groups copies printed one
/// after another (GCode.cpp `_do_export`, ToolOrdering.cpp
/// `group_by_plate_map`: the manual mode, nothing unprintable).
pub(crate) fn group(
    settings: &Settings,
    layers: Vec<Vec<u32>>,
    objects: &[(Vec<[f64; 2]>, f64)],
    tower: Option<[f64; 4]>,
    by_plate_map: bool,
) -> Result<Grouped, Failure> {
    let mut settings = merged(settings);
    // A rack pin on an automatic map: the pinned filament's extruder one
    // that can take its pin (`crate::rack::settle_automatic`, done before
    // the walls are laid too; it changes nothing a second time).
    crate::rack::settle_automatic(&mut settings);
    let settings = &settings;
    let used: Vec<u32> = (layers.iter().flatten().copied())
        .collect::<BTreeSet<u32>>()
        .into_iter()
        .collect();
    // Print.cpp l.3442: a rack the slice cannot honour is refused first,
    // a pin no extruder can take on an automatic map before that.
    crate::rack::check_automatic(settings, &used)?;
    crate::rack::validate(settings, &used)?;
    let machine = machine(settings)?;
    let filament_count = settings.texts("filament_colour").map_or(0, <[String]>::len);
    let mode_name = if by_plate_map {
        "Manual"
    } else {
        settings
            .text("filament_map_mode")
            .unwrap_or("Auto For Flush")
    };
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
        geometric: {
            let mut sets = geometric(settings, objects, tower)?;
            // Rule P: on an automatic map, a pinned filament barred from
            // the extruders that cannot take its pin (`rack::barred`).
            if mode != Mode::Manual {
                let bars = crate::rack::barred(settings, &used, machine.extruders);
                for (extruder, bar) in bars.iter().enumerate() {
                    for filament in bar {
                        if let Some(set) = sets.get_mut(extruder) {
                            set.insert(*filament as i32);
                        }
                    }
                }
                for filament in &used {
                    let f = *filament as i32;
                    if bars.iter().any(|bar| bar.contains(filament))
                        && sets.iter().all(|set| set.contains(&f))
                    {
                        let racks = crate::rack::racks(settings);
                        return Err(format!(
                            "Filament {} is pinned to a {} mm rack nozzle, but no extruder that \
                             has one can print it.; no G-code was written",
                            f + 1,
                            crate::rack::size_text(crate::rack::size_pin(
                                settings,
                                &racks,
                                *filament as usize
                            ))
                        )
                        .into());
                    }
                }
            }
            sets
        },
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
    crate::rack::check_pins(settings, mode == Mode::Manual)?;
    let mut grouping = edge_group::group_filaments(&machine, &input)?;
    // ToolOrdering.cpp l.1985 and l.2155: on a printer with a rack, every
    // filament onto a nozzle of the size it prints with and of the slot
    // it is pinned to, and checked so.
    if machine.multi_nozzle
        && let Some(grouping) = &mut grouping
    {
        if crate::rack::pin(settings, &mut grouping.result) > 0 {
            grouping.filament_map = grouping.result.extruder_map(false);
        }
        crate::rack::check(settings, &grouping.result)?;
        crate::rack::check_one_size(settings, &grouping.result)?;
    }
    Ok(Grouped {
        machine,
        input,
        flush_values,
        flush_multiplier,
        flush_fast,
        grouping,
        orders: Vec::new(),
    })
}

/// Each layer's filaments in the order the layer prints them, put in
/// `grouped` once the layers are put in order (which reads the grouping):
/// with `preload_all_filaments`, the first layer loads every other
/// filament ahead of its own (ToolOrdering.cpp `apply_preload_all_filaments`).
pub(crate) fn set_orders(
    settings: &Settings,
    grouped: &mut Grouped,
    mut orders: Vec<Vec<u32>>,
) -> Result<(), Failure> {
    let settings = &merged(settings);
    if let Some(grouping) = &grouped.grouping
        && settings.flag("preload_all_filaments")? == Some(true)
        && settings.flag("enable_prime_tower")? == Some(true)
        && let Some(first) = orders.iter_mut().find(|order| !order.is_empty())
    {
        let extruders = grouping.result.extruder_map(true);
        *first = edge_group::preload_first_layer_tool_order(first, &grouping.result.used, |f| {
            extruders.get(f as usize).copied().unwrap_or(0)
        });
    }
    grouped.orders = orders;
    Ok(())
}

/// What the G-code writer is told of a grouping: each of the project's
/// filaments' extruder from 1 (GCode.cpp `_do_export`: the grouping's
/// `get_extruder_map(false)`, 1 where it names none), nozzle (`get_nozzle_map`)
/// and, for the file's settings, the kind of nozzle each filament the plate
/// uses landed on over the project's `filament_volume_map`
/// (`grouping_header_maps`).
pub(crate) fn nozzle_map(settings: &Settings, grouping: &edge_group::Grouping) -> NozzleMap {
    let count = (settings.texts("filament_colour").map_or(0, <[String]>::len))
        .max(grouping.filament_map.len())
        .max(1);
    let extruders = (0..count)
        .map(|filament| match grouping.filament_map.get(filament) {
            Some(extruder) if *extruder >= 1 => *extruder as u32,
            _ => 1,
        })
        .collect();
    let mut volumes: Vec<i32> = (settings.texts("filament_volume_map").unwrap_or_default())
        .iter()
        .map(|value| value.trim().parse::<i32>().unwrap_or(0))
        .collect();
    let filaments = settings.texts("filament_colour").map_or(0, <[String]>::len);
    volumes.resize(filaments.max(1), VolumeType::STANDARD.0);
    let result = &grouping.result;
    for filament in &result.used {
        if let (Some(slot), Some(nozzle)) = (
            volumes.get_mut(*filament as usize),
            result.nozzle_for(i64::from(*filament)),
        ) {
            *slot = nozzle.volume_type.0;
        }
    }
    NozzleMap {
        extruders,
        nozzles: result.nozzle_map(),
        volumes,
    }
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

    /// What the writer is told: every filament of the project, an unused
    /// one on the first extruder; the nozzles from 0; the kind of nozzle a
    /// used filament landed on, the project's own for the rest.
    #[test]
    fn the_writer_is_told_each_filaments_extruder_and_nozzle() {
        let mut settings = h2d();
        settings.insert(
            "filament_colour",
            edge_config::Value::Many(vec!["#FFFFFF".into(), "#202020".into(), "#C12E1F".into()]),
        );
        settings.insert(
            "flush_volumes_matrix",
            edge_config::Value::Many(
                ["0", "1", "2", "3", "0", "4", "5", "6", "0"]
                    .map(str::to_owned)
                    .to_vec(),
            ),
        );
        settings.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        settings.insert(
            "filament_map",
            edge_config::Value::Many(vec!["2".into(), "1".into(), "2".into()]),
        );
        settings.insert(
            "nozzle_volume_type",
            edge_config::Value::Many(vec!["Standard".into(), "High Flow".into()]),
        );
        settings.insert(
            "filament_volume_map",
            edge_config::Value::Many(vec!["0".into(), "0".into(), "5".into()]),
        );
        let grouped = group(&settings, vec![vec![0, 1]; 2], &[], None, false).unwrap();
        let map = nozzle_map(&settings, grouped.grouping.as_ref().unwrap());
        assert_eq!(map.extruders, [2, 1, 2]);
        assert_eq!(map.nozzles, [1, 0, 1]);
        // Filament 0 on the High Flow nozzle, 1 on the Standard one, 2
        // unused keeps its own.
        assert_eq!(map.volumes, [1, 0, 5]);
        assert_eq!(map.extruder(2), 1);
        assert_eq!(map.extruder(7), 0);
    }

    #[test]
    fn two_filaments_on_an_h2d_go_one_to_each_nozzle() {
        let grouped = group(&h2d(), vec![vec![0, 1]; 3], &[], None, false).unwrap();
        // Either way round costs the same, and the first tried wins:
        // filament 1 on the right nozzle. The reference, reading the one
        // matrix as half a matrix to each nozzle, gives [1, 2]
        // (work/bambu/h2d_two.3mf).
        assert_eq!(grouped.grouping.unwrap().filament_map, [2, 1]);
    }

    fn many(values: &[&str]) -> edge_config::Value {
        edge_config::Value::Many(values.iter().map(|v| (*v).to_owned()).collect())
    }

    #[test]
    fn the_h2c_rack_is_grouped_onto() {
        let mut rack = h2d();
        rack.insert("extruder_max_nozzle_count", many(&["1", "6"]));
        // The machine's own rack: the left nozzle and six on the right.
        let machine = machine(&merged(&rack)).unwrap();
        assert!(machine.multi_nozzle);
        assert_eq!(machine.nozzles.len(), 7);
        assert!(machine.nozzles[1..].iter().all(|n| n.extruder_id == 1));
        // One filament on each side prints as on two nozzles.
        rack.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        rack.insert("filament_map", many(&["1", "2"]));
        let grouped = group(&rack, vec![vec![0, 1]; 3], &[], None, false).unwrap();
        assert_eq!(grouped.grouping.unwrap().result.nozzle_map(), [0, 1]);
        // Both on the right with a flush between them: the grouping puts
        // them on two of the rack's nozzles, which with the tower off are
        // changed between by the change G-code alone.
        rack.insert("filament_map", many(&["2", "2"]));
        rack.insert(
            "flush_volumes_matrix",
            many(&["0", "199", "639", "0", "0", "199", "639", "0"]),
        );
        rack.insert("enable_prime_tower", edge_config::Value::One("0".into()));
        let grouped = group(&rack, vec![vec![0, 1]; 3], &[], None, false).unwrap();
        assert_eq!(grouped.grouping.unwrap().result.nozzle_map(), [2, 1]);
        // With the tower on too, which rams the outgoing filament first.
        let mut tower = rack.clone();
        tower.insert("enable_prime_tower", edge_config::Value::One("1".into()));
        let grouped = group(&tower, vec![vec![0, 1]; 3], &[], None, false).unwrap();
        assert_eq!(grouped.grouping.unwrap().result.nozzle_map(), [2, 1]);
        // Rack hotends taken as extruders (H3C) are refused.
        let mut h3c = h2d();
        h3c.insert("nozzle_diameter", many(&["0.4"]));
        h3c.insert("extruder_max_nozzle_count", many(&["9"]));
        let said = group(&h3c, vec![vec![0]], &[], None, false)
            .unwrap_err()
            .to_string();
        assert!(said.contains("H3C"), "{said}");
    }

    /// A rack set by hand: the project's text over the printer's, its
    /// nozzles numbered by physical slot (ToolSlots.cpp `grouping_nozzle_list`),
    /// each filament moved onto a nozzle of the size it is pinned to
    /// (work/tower/h2c3: h2c3_mix2_211_pn, the reference's map 2,0,0).
    #[test]
    fn a_rack_set_by_hand_is_grouped_onto_by_slot_and_size() {
        let mut rack = h2d();
        rack.insert("extruder_max_nozzle_count", many(&["1", "6"]));
        rack.insert("extruder_rack_nozzle_diameter", many(&["", "0.4,0.4"]));
        rack.insert(
            "extruder_rack_nozzle_override",
            many(&["", "0.4,0.6,0.4,0.6"]),
        );
        let machine = machine(&merged(&rack)).unwrap();
        let sizes: Vec<&str> = machine
            .nozzles
            .iter()
            .map(|n| n.diameter.as_str())
            .collect();
        assert_eq!(sizes, ["0.4", "0.4", "0.6", "0.4", "0.6"]);
        let ids: Vec<i32> = machine.nozzles.iter().map(|n| n.group_id).collect();
        assert_eq!(ids, [0, 1, 2, 3, 4]);
        rack.insert("filament_colour", many(&["#FFFFFF", "#202020", "#C12E1F"]));
        rack.insert("filament_diameter", many(&["1.75", "1.75", "1.75"]));
        rack.insert(
            "flush_volumes_matrix",
            many(&["0", "199", "199", "639", "0", "199", "639", "639", "0"]),
        );
        rack.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        rack.insert("filament_map", many(&["2", "1", "1"]));
        rack.insert("filament_rack_nozzle_diameter", many(&["0.6", "0", "0"]));
        let grouped = group(&rack, vec![vec![0, 1, 2]; 3], &[], None, false).unwrap();
        let grouping = grouped.grouping.unwrap();
        assert_eq!(grouping.result.nozzle_map(), [2, 0, 0]);
        assert_eq!(grouping.filament_map, [2, 1, 1]);
        // Both sizes on the right: refused (the reference warns).
        rack.insert("filament_map", many(&["2", "2", "1"]));
        let said = group(&rack, vec![vec![0, 1, 2]; 3], &[], None, false)
            .unwrap_err()
            .to_string();
        assert!(
            said.contains("two nozzle diameters on the same extruder rack"),
            "{said}"
        );
        // A pin on an automatic map (rule P): the saved map puts filament 1
        // on the left, which has no 0.6; it goes right, onto a 0.6, the
        // other filament where the grouping wants it (h2c4_two_auto_pn).
        rack.insert(
            "filament_map_mode",
            edge_config::Value::One("Auto For Flush".into()),
        );
        rack.insert("filament_map", many(&["1", "1", "1"]));
        let grouped = group(&rack, vec![vec![0, 1]; 3], &[], None, false).unwrap();
        let grouping = grouped.grouping.unwrap();
        assert_eq!(grouping.filament_map[0], 2);
        assert_eq!(grouping.result.nozzle_map()[0], 2);
        // A pin no extruder can take: refused (E1).
        rack.insert("filament_rack_nozzle_diameter", many(&["0.8", "0", "0"]));
        let said = group(&rack, vec![vec![0, 1]; 3], &[], None, false)
            .unwrap_err()
            .to_string();
        assert!(said.contains("but no extruder has one"), "{said}");
    }

    /// Copies printed one after another take the plate's own map, the
    /// grouping mode notwithstanding (work/tower/h2, h2_seq_auto: the
    /// reference writes `filament_map = 1,1` with Auto For Flush).
    #[test]
    fn objects_one_after_another_take_the_plates_map() {
        let mut settings = h2d();
        settings.insert(
            "filament_map",
            edge_config::Value::Many(vec!["1".into(), "1".into()]),
        );
        let grouped = group(&settings, vec![vec![0, 1]; 3], &[], None, true).unwrap();
        let grouping = grouped.grouping.unwrap();
        assert_eq!(grouping.filament_map, [1, 1]);
        assert_eq!(nozzle_map(&settings, &grouping).nozzles, [0, 0]);
    }

    /// Nozzles of two sizes with a map set by hand group as the map says
    /// (`crate::nozzle_process` gives each nozzle its own size's process).
    #[test]
    fn nozzles_of_two_sizes_keep_the_map_set_by_hand() {
        let mut mixed = h2d();
        mixed.insert(
            "nozzle_diameter",
            edge_config::Value::Many(vec!["0.4".into(), "0.6".into()]),
        );
        mixed.insert(
            "filament_map_mode",
            edge_config::Value::One("Manual".into()),
        );
        mixed.insert(
            "filament_map",
            edge_config::Value::Many(vec!["2".into(), "1".into()]),
        );
        let grouped = group(&mixed, vec![vec![0, 1]], &[], None, false).unwrap();
        assert_eq!(grouped.grouping.unwrap().filament_map, [2, 1]);
    }
}
