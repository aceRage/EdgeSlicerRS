//! A sliced file (`.gcode.3mf`) of a project: "Export plate sliced file"
//! and "Export all plate sliced file" (Plater.cpp `export_gcode_3mf`).
//! What the slice came to is read from the slice and from the G-code as
//! written, as the reference reads its own (bbs_3mf.cpp
//! `PlateData::parse_filament_info`, PartPlate.cpp `store_to_3mf_structure`,
//! Plater.cpp `generate_first_layer_bbox`); [`edge_3mf::write_sliced`]
//! lays it out.

use std::{collections::BTreeMap, path::Path};

use edge_3mf::{
    ObjectBox, Pictures, PlateBoxes, SliceFilament, SliceInfo, SliceNozzle, SlicedFile, SlicedPlate,
};
use edge_config::Settings;

use crate::{Failure, Model, Saved, Saving, Sliced, toolpath};

/// One plate sliced, as it goes in a sliced file.
pub struct PlateSlice<'a> {
    /// Its number, from 1.
    pub plate: u32,
    pub sliced: &'a Sliced,
    pub gcode: &'a [u8],
    /// How long it prints, in seconds, as the G-code says.
    pub seconds: f64,
    /// The printer model's id in its vendor's files ("N1" for the Bambu
    /// Lab A1 mini; `Library::model_id`), which a Bambu printer and Bambu
    /// Connect match the file by: `printer_model_id` (Plater.cpp
    /// l.26748). Empty when the caller has no profiles to look in.
    pub printer_model_id: Option<String>,
}

/// The extruder each of `filaments` prints from, from 1, as the reference
/// reports it (bbs_3mf.cpp `reported_filament_maps`): the settings'
/// `filament_map`, except on a printer of several toolheads alike
/// (PrintConfig.cpp `is_identical_multi_extruder_printer`), where
/// filament i prints from toolhead i, round again past the last.
pub fn filament_maps(settings: &Settings, filaments: usize) -> Vec<u32> {
    let extruders = settings.texts("nozzle_diameter").map_or(1, <[String]>::len);
    let together = settings
        .texts("single_extruder_multi_material")
        .and_then(<[String]>::first)
        .is_some_and(|value| value == "1");
    let variants = settings.texts("extruder_variant_list").unwrap_or_default();
    let differ = extruders <= 2 && {
        let kinds: std::collections::BTreeSet<&str> = variants
            .iter()
            .take(extruders)
            .flat_map(|list| list.split(',').map(str::trim))
            .filter(|kind| !kind.is_empty())
            .collect();
        kinds.len() > 1
    };
    let identical = extruders >= 2 && !together && !differ;
    let mut maps: Vec<u32> = settings
        .texts("filament_map")
        .unwrap_or_default()
        .iter()
        .map(|value| value.trim().parse::<u32>().unwrap_or(1).max(1))
        .collect();
    if identical && maps.iter().all(|value| *value <= 1) {
        maps = (0..filaments)
            .map(|at| (at % extruders) as u32 + 1)
            .collect();
    }
    maps.resize(filaments.max(maps.len()), 1);
    maps.truncate(filaments.max(1));
    maps
}

/// The bed's surface as the G-code names it (PrintConfig.hpp
/// `bed_type_to_gcode_string`).
fn bed_name(bed: &str) -> &'static str {
    match bed {
        "Supertack Plate" => "supertack_plate",
        "Cool Plate" => "cool_plate",
        "Textured Cool Plate" => "textured_cool_plate",
        "Engineering Plate" => "eng_plate",
        "High Temp Plate" => "hot_plate",
        "Textured PEI Plate" => "textured_plate",
        _ => "unknown",
    }
}

/// What the G-code's notes say of a print: each filament's length in mm
/// and weight in grams, by number from 1; the whole weight; how long the
/// first layer takes; whether supports print on the tower.
#[derive(Clone, Debug, Default, PartialEq)]
struct Notes {
    used_mm: Vec<f64>,
    used_g: Vec<f64>,
    total_g: Option<f64>,
    first_layer_seconds: f64,
    support_on_tower: bool,
}

/// "1d 2h 3m 4s" in seconds.
fn duration(text: &str) -> Option<f64> {
    let mut seconds = 0.0;
    for part in text.split_whitespace() {
        let (number, unit) = part.split_at(part.len().checked_sub(1)?);
        let number: f64 = number.parse().ok()?;
        seconds += number
            * match unit {
                "d" => 86_400.0,
                "h" => 3_600.0,
                "m" => 60.0,
                "s" => 1.0,
                _ => return None,
            };
    }
    Some(seconds)
}

fn notes(gcode: &[u8]) -> Notes {
    let mut notes = Notes::default();
    let numbers = |text: &str| -> Vec<f64> {
        text.split(',')
            .map(|value| value.trim().parse().unwrap_or(0.0))
            .collect()
    };
    for line in gcode.split(|byte| *byte == b'\n') {
        let Some(line) = line.strip_prefix(b"; ") else {
            continue;
        };
        let Ok(line) = std::str::from_utf8(line) else {
            continue;
        };
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("filament used [mm] = ") {
            notes.used_mm = numbers(rest);
        } else if let Some(rest) = line.strip_prefix("filament used [g] = ") {
            notes.used_g = numbers(rest);
        } else if let Some(rest) = line.strip_prefix("total filament used [g] = ") {
            notes.total_g = rest.trim().parse().ok();
        } else if let Some(rest) =
            line.strip_prefix("estimated first layer printing time (normal mode) = ")
        {
            notes.first_layer_seconds = duration(rest).unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("support_material_on_wipe_tower: ") {
            notes.support_on_tower = rest.trim() == "1";
        }
    }
    notes
}

/// An enum list of the settings as numbers, one for each extruder.
fn per_extruder(settings: &Settings, key: &str, names: &[&str], extruders: usize) -> Vec<i32> {
    let mut values: Vec<i32> = settings
        .texts(key)
        .unwrap_or_default()
        .iter()
        .map(|value| {
            names
                .iter()
                .position(|name| name == value)
                .map_or_else(|| value.parse().unwrap_or(0), |at| at as i32)
        })
        .collect();
    if values.is_empty() {
        values.push(0);
    }
    let last = *values.last().unwrap_or(&0);
    values.resize(extruders.max(values.len()), last);
    values
}

/// One plate's part of a sliced file. `copies`: the number and name of
/// each printable copy on the plate, in the plate's order, as
/// [`Sliced::first_layer_objects`] lists them.
pub fn sliced_plate<'a>(slice: &PlateSlice<'a>, copies: &[(u32, String)]) -> SlicedPlate<'a> {
    let settings = &slice.sliced.settings;
    let first = |key: &str| {
        settings
            .texts(key)
            .and_then(<[String]>::first)
            .cloned()
            .unwrap_or_default()
    };
    let flag = |key: &str| first(key) == "1" || first(key) == "true";
    let diameters: Vec<f64> = settings
        .texts("nozzle_diameter")
        .unwrap_or_default()
        .iter()
        .map(|value| value.parse().unwrap_or(0.4))
        .collect();
    let diameters = if diameters.is_empty() {
        vec![0.4]
    } else {
        diameters
    };
    let colours = settings.texts("filament_colour").unwrap_or_default();
    let filaments = colours.len().max(1);
    let maps = filament_maps(settings, filaments);
    let notes = notes(slice.gcode);

    // What each filament laid, for objects and for supports.
    let path = toolpath::read_toolpath(slice.gcode);
    let mut for_object = vec![false; filaments + 1];
    let mut for_support = vec![false; filaments + 1];
    let mut sequence: Vec<u32> = Vec::new();
    for step in &path.moves {
        let at = usize::from(step.filament).min(filaments);
        if step.kind != toolpath::MoveKind::Extrude {
            continue;
        }
        use toolpath::Feature as F;
        match step.feature {
            F::Support | F::SupportInterface | F::SupportTransition => {
                for_support[at] = true;
            }
            F::None | F::Custom | F::PrimeTower | F::Skirt | F::Brim => {}
            _ => for_object[at] = true,
        }
        // Each filament as it is taken up, the first one too.
        if sequence.last() != Some(&u32::from(step.filament)) {
            sequence.push(u32::from(step.filament));
        }
    }
    // One filament taken up is no order to keep (GCode.cpp records a
    // change only where there is one).
    if sequence
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        < 2
    {
        sequence.clear();
    }

    let text = |key: &str, at: usize| {
        settings
            .texts(key)
            .and_then(|values| values.get(at).or(values.first()))
            .cloned()
            .unwrap_or_default()
    };
    let mut used = Vec::new();
    for at in 0..filaments {
        let grams = notes.used_g.get(at).copied().unwrap_or(0.0);
        let mm = notes.used_mm.get(at).copied().unwrap_or(0.0);
        if mm <= 0.0 && grams <= 0.0 {
            continue;
        }
        let extruder = maps.get(at).copied().unwrap_or(1).max(1);
        used.push(SliceFilament {
            id: at as u32 + 1,
            tray_info_idx: text("filament_ids", at),
            kind: text("filament_type", at),
            colour: text("filament_colour", at),
            used_m: mm / 1000.0,
            used_g: grams,
            group_id: extruder - 1,
            nozzle_diameter: diameters
                .get(extruder as usize - 1)
                .copied()
                .unwrap_or(diameters[0]),
            volume_type: "Standard".into(),
            used_for_object: for_object[at + 1],
            used_for_support: for_support[at + 1],
        });
    }
    // Every toolhead the map names, on a printer of several alike
    // (bbs_3mf.cpp: "Build the same list from the reported map").
    let mut nozzles = Vec::new();
    if diameters.len() >= 2 {
        let named: std::collections::BTreeSet<u32> = maps.iter().copied().collect();
        for extruder in named {
            nozzles.push(SliceNozzle {
                id: extruder - 1,
                extruder_id: extruder,
                diameter: diameters
                    .get(extruder as usize - 1)
                    .copied()
                    .unwrap_or(diameters[0]),
                volume_type: "Standard".into(),
            });
        }
    }
    let extruders = diameters.len();
    let info = SliceInfo {
        extruder_types: per_extruder(
            settings,
            "extruder_type",
            &["Direct Drive", "Bowden"],
            extruders,
        ),
        nozzle_volume_types: per_extruder(
            settings,
            "nozzle_volume_type",
            &["Standard", "High Flow"],
            extruders,
        ),
        printer_model_id: slice.printer_model_id.clone().unwrap_or_default(),
        nozzle_diameters: diameters
            .iter()
            .map(|value| format!("{value}"))
            .collect::<Vec<_>>()
            .join(","),
        timelapse_type: first("timelapse_type").parse().unwrap_or(0),
        prediction: slice.seconds.max(0.0) as u64,
        // The print's whole weight (`PrintStatistics::total_weight`): the
        // G-code's total where it writes one, else the filaments' grams
        // added up (a Bambu printer's G-code writes no total line).
        weight: notes
            .total_g
            .or_else(|| Some(notes.used_g.iter().sum()))
            .filter(|grams| *grams != 0.0),
        first_layer_time: notes.first_layer_seconds,
        support_used: flag("enable_support"),
        // Objects are not labelled for cancelling (edge-gcode notes it).
        label_object_enabled: false,
        support_material_on_wipe_tower: notes.support_on_tower,
        has_filament_switcher: flag("has_filament_switcher"),
        filament_maps: maps.clone(),
        objects: copies.to_vec(),
        filaments: used,
        nozzles,
        sequence,
    };

    // The first layer's boxes: each copy's, and the tower's.
    let layer_height = slice.sliced.height_of(0) as f32;
    let mut objects: Vec<ObjectBox> = slice
        .sliced
        .first_layer_objects()
        .into_iter()
        .zip(copies)
        .filter_map(|(first, (id, name))| {
            let first = first?;
            Some(ObjectBox {
                id: *id,
                bbox: first.bbox,
                area: first.area as f32,
                layer_height,
                name: name.clone(),
            })
        })
        .collect();
    if let Some(tower) = slice.sliced.tower.footprint {
        objects.push(ObjectBox {
            // PartPlate's index from 0, past 1000 (Plater.cpp).
            id: slice.plate - 1 + 1000,
            bbox: tower,
            area: 0.0,
            layer_height,
            name: "wipe_tower".into(),
        });
    }
    let boxes = (!objects.is_empty()).then(|| {
        let mut all = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for object in &objects {
            all = [
                all[0].min(object.bbox[0]),
                all[1].min(object.bbox[1]),
                all[2].max(object.bbox[2]),
                all[3].max(object.bbox[3]),
            ];
        }
        let filament_ids: Vec<u32> = slice
            .sliced
            .used
            .iter()
            .map(|filament| *filament as u32 - 1)
            .collect();
        let first_extruder = path
            .moves
            .iter()
            .find(|step| step.kind == toolpath::MoveKind::Extrude)
            .map_or(0, |step| u32::from(step.filament).saturating_sub(1));
        PlateBoxes {
            all,
            filament_colours: filament_ids
                .iter()
                .map(|at| text("filament_colour", *at as usize))
                .collect(),
            filament_ids,
            is_seq_print: first("print_sequence") == "by object",
            first_extruder,
            nozzle_diameter: diameters
                .get(maps.get(first_extruder as usize).copied().unwrap_or(1) as usize - 1)
                .copied()
                .unwrap_or(diameters[0]) as f32,
            bed_type: bed_name(&first("curr_bed_type")).to_owned(),
            first_layer_time: notes.first_layer_seconds as f32,
            objects,
        }
    });
    SlicedPlate {
        plate: slice.plate,
        gcode: slice.gcode,
        info,
        boxes,
    }
}

impl Model {
    /// Each copy's number in a sliced file (its `identify_id`), by the
    /// copy's id: what its pick picture is drawn in.
    pub fn copy_numbers(&self) -> Vec<(u64, u32)> {
        let project = self.as_project(None, None);
        let numbers = edge_3mf::identify_numbers(&project);
        let mut out: Vec<(u64, u32)> = self
            .objects()
            .iter()
            .enumerate()
            .flat_map(|(at, object)| {
                let numbers = &numbers;
                object
                    .copies()
                    .iter()
                    .enumerate()
                    .filter_map(move |(copy, placed)| {
                        Some((placed.id(), *numbers.get(&(at, copy))?))
                    })
            })
            .collect();
        out.sort_unstable();
        out
    }

    /// Writes a sliced file of the plates in `slices` to `path`, whole or
    /// not at all: the project's settings as [`Model::save`] writes them
    /// (`saving`), its plates, and each plate's G-code with what goes
    /// with it. `plate`: the plate exported, or none for every plate
    /// sliced. `pictures`: each plate's, by number, drawn by the caller.
    pub fn save_sliced(
        &self,
        path: &Path,
        saving: &Saving,
        slices: &[PlateSlice<'_>],
        plate: Option<u32>,
        pictures: BTreeMap<u32, Pictures>,
    ) -> Result<Saved, Failure> {
        let (text, settings) = self.saving_text(saving)?;
        let project = self.as_project(None, settings.as_ref());
        let identify = edge_3mf::identify_numbers(&project);
        let filaments = settings
            .as_ref()
            .and_then(|settings| settings.texts("filament_colour").map(<[String]>::len))
            .unwrap_or(1);
        // The printable copies of each plate, in its order, as the slice
        // has them.
        let copies_of = |id: u32| -> Vec<(u32, String)> {
            let refs: Vec<edge_3mf::InstanceRef> = match project.plates.iter().find(|p| p.id == id)
            {
                Some(plate) => plate.instances.clone(),
                None => (project.objects.iter().enumerate())
                    .flat_map(|(object, o)| {
                        (0..o.instances.len())
                            .map(move |instance| edge_3mf::InstanceRef { object, instance })
                    })
                    .collect(),
            };
            refs.iter()
                .filter_map(|at| {
                    let object = project.objects.get(at.object)?;
                    let copy = object.instances.get(at.instance)?;
                    let printed = object.parts.iter().any(|part| {
                        part.kind == edge_3mf::PartKind::Normal && !part.mesh.triangles.is_empty()
                    });
                    (copy.printable && printed).then(|| {
                        (
                            identify
                                .get(&(at.object, at.instance))
                                .copied()
                                .unwrap_or(0),
                            object.name.clone().unwrap_or_default(),
                        )
                    })
                })
                .collect()
        };
        let plates: Vec<SlicedPlate<'_>> = slices
            .iter()
            .map(|slice| sliced_plate(slice, &copies_of(slice.plate)))
            .collect();
        let maps = slices
            .first()
            .map(|slice| filament_maps(&slice.sliced.settings, filaments))
            .unwrap_or_default();
        let file = SlicedFile {
            plate,
            plates,
            pictures,
            filament_maps: maps,
        };
        let save = edge_3mf::Save {
            project: &project,
            settings: text,
            presets: saving.presets(),
            inside: self.inside.as_deref().map(Vec::as_slice),
            filaments,
        };
        let written = edge_3mf::write_sliced(&save, &file, path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Saved {
            bytes: written.bytes,
            entries: written.entries,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(pairs: &[(&str, &[&str])]) -> Settings {
        let mut settings = Settings::default();
        for (key, values) in pairs {
            settings.insert(
                *key,
                edge_config::Value::Many(values.iter().map(|v| (*v).to_owned()).collect()),
            );
        }
        settings
    }

    #[test]
    fn a_printer_of_toolheads_alike_prints_each_filament_from_its_own() {
        // The reference's slice_info of mcbrim_test.3mf on a Snapmaker U1.
        let u1 = settings(&[
            ("nozzle_diameter", &["0.4", "0.4", "0.4", "0.4"]),
            ("filament_map", &["1", "1", "1", "1", "1", "1", "1"]),
        ]);
        assert_eq!(filament_maps(&u1, 7), [1, 2, 3, 4, 1, 2, 3]);
        let one = settings(&[("nozzle_diameter", &["0.4"]), ("filament_map", &["1", "1"])]);
        assert_eq!(filament_maps(&one, 2), [1, 1]);
        let mixed = settings(&[
            ("nozzle_diameter", &["0.4", "0.4"]),
            (
                "extruder_variant_list",
                &["Direct Drive Standard", "Bowden Standard"],
            ),
            ("filament_map", &["1", "2"]),
        ]);
        assert_eq!(filament_maps(&mixed, 2), [1, 2]);
    }

    #[test]
    fn the_gcode_notes_are_read() {
        let gcode = b"; HEADER_BLOCK_START\n; support_material_on_wipe_tower: 1\n; HEADER_BLOCK_END\nG1 X1\n; filament used [mm] = 0.00, 667.41\n; filament used [g] = 0.00, 2.20\n; total filament used [g] = 2.20\n; estimated first layer printing time (normal mode) = 1h 11m 13s\n";
        let read = notes(gcode);
        assert_eq!(read.used_mm, [0.0, 667.41]);
        assert_eq!(read.used_g, [0.0, 2.2]);
        assert_eq!(read.total_g, Some(2.2));
        assert_eq!(read.first_layer_seconds, 3600.0 + 673.0);
        assert!(read.support_on_tower);
    }
}
