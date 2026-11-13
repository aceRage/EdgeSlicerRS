//! What a sliced file (`.gcode.3mf`) holds beyond the project: each
//! plate's G-code and its MD5, its pictures, the first layer's boxes
//! (`Metadata/plate_N.json`), what the slice came to
//! (`Metadata/slice_info.config`) and the order filaments were taken up
//! in (`Metadata/filament_sequence.json`), as the reference's "Export
//! plate sliced file" and "Export all plate sliced file" write them
//! (Plater.cpp `export_gcode_3mf`: `export_3mf` with
//! `SplitModel | WithGcode | SkipModel`; bbs_3mf.cpp
//! `_add_slice_info_config_file_to_archive`, `_add_bbox_file_to_archive`,
//! `_add_filament_sequence_file_to_archive`, `_add_gcode_file_to_archive`).
//!
//! The model itself is left out (`SkipModel`): the file is for a printer,
//! not for opening as a project. [`crate::write_sliced`] writes it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use md5::{Digest, Md5};

use crate::write::{REFERENCE_VERSION, escape};

/// A sliced file to write.
#[derive(Clone, Debug, Default)]
pub struct SlicedFile<'a> {
    /// The plate exported ("Export plate sliced file"), or none for every
    /// plate sliced ("Export all plate sliced file").
    pub plate: Option<u32>,
    /// Each plate sliced that goes in it, in plate order.
    pub plates: Vec<SlicedPlate<'a>>,
    /// Each plate's pictures, by plate number. The reference draws them
    /// for every plate whichever is exported; a plate without them is
    /// written without.
    pub pictures: BTreeMap<u32, Pictures>,
    /// The extruder each filament prints from, from 1, as every plate's
    /// settings name it; empty for one each from extruder 1.
    pub filament_maps: Vec<u32>,
}

/// One plate's pictures, as PNG files, as the reference draws them
/// (`Plater::export_3mf`: 512 × 512, the small one 128 × 128).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pictures {
    /// The plate from the reference's default view, lit
    /// (`Metadata/plate_N.png`); and the same at a quarter of the size
    /// (`Metadata/plate_N_small.png`).
    pub plate: Vec<u8>,
    pub small: Vec<u8>,
    /// The same view without light (`Metadata/plate_no_light_N.png`).
    pub no_light: Vec<u8>,
    /// From above (`Metadata/top_N.png`), and from above with each copy in
    /// the colour of its number (`Metadata/pick_N.png`): what a printer
    /// that skips objects shows them by.
    pub top: Vec<u8>,
    pub pick: Vec<u8>,
}

/// One plate sliced.
#[derive(Clone, Debug, Default)]
pub struct SlicedPlate<'a> {
    /// Its number, from 1.
    pub plate: u32,
    pub gcode: &'a [u8],
    pub info: SliceInfo,
    /// Its first layer's boxes; none when it has nothing on it.
    pub boxes: Option<PlateBoxes>,
}

/// What a plate's slice came to, as `Metadata/slice_info.config` has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SliceInfo {
    /// Each extruder's kind and nozzle (`extruder_type`,
    /// `nozzle_volume_type`), as numbers: one for each extruder.
    pub extruder_types: Vec<i32>,
    pub nozzle_volume_types: Vec<i32>,
    /// The printer's model, for Bambu Lab printers; empty for others.
    pub printer_model_id: String,
    /// Each extruder's nozzle size, as written (`0.4,0.4`).
    pub nozzle_diameters: String,
    pub timelapse_type: i32,
    /// How long it prints, in whole seconds.
    pub prediction: u64,
    /// What it weighs in grams; none when nothing was weighed.
    pub weight: Option<f64>,
    /// How long its first layer takes, in seconds.
    pub first_layer_time: f64,
    pub support_used: bool,
    pub label_object_enabled: bool,
    pub support_material_on_wipe_tower: bool,
    pub has_filament_switcher: bool,
    /// The extruder each filament prints from, from 1.
    pub filament_maps: Vec<u32>,
    /// Each printable copy on it: its number and its object's name.
    pub objects: Vec<(u32, String)>,
    pub filaments: Vec<SliceFilament>,
    pub nozzles: Vec<SliceNozzle>,
    /// The filaments in the order they are taken up, from 1; empty for a
    /// plate printed in one.
    pub sequence: Vec<u32>,
}

/// One filament a plate prints with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SliceFilament {
    /// Its number, from 1.
    pub id: u32,
    /// Its preset's filament id (`filament_ids`, the AMS tray's).
    pub tray_info_idx: String,
    pub kind: String,
    pub colour: String,
    pub used_m: f64,
    pub used_g: f64,
    /// The nozzle it is printed from, from 0.
    pub group_id: u32,
    pub nozzle_diameter: f64,
    pub volume_type: String,
    pub used_for_object: bool,
    pub used_for_support: bool,
}

/// One nozzle a plate prints from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SliceNozzle {
    /// From 0.
    pub id: u32,
    /// From 1.
    pub extruder_id: u32,
    pub diameter: f64,
    pub volume_type: String,
}

/// The first layer's boxes of a plate, as `Metadata/plate_N.json` has
/// them (ThumbnailData.hpp `PlateBBoxData`): in mm, on the plate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlateBoxes {
    /// Of everything on the first layer: least x, y, greatest x, y.
    pub all: [f64; 4],
    pub objects: Vec<ObjectBox>,
    /// The filaments used, from 0, and their colours.
    pub filament_ids: Vec<u32>,
    pub filament_colours: Vec<String>,
    pub is_seq_print: bool,
    /// The filament printed first, from 0.
    pub first_extruder: u32,
    pub nozzle_diameter: f32,
    /// The bed's surface as the G-code names it (`textured_plate`).
    pub bed_type: String,
    pub first_layer_time: f32,
}

/// One object's first layer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObjectBox {
    pub id: u32,
    pub bbox: [f64; 4],
    pub area: f32,
    pub layer_height: f32,
    pub name: String,
}

/// The MD5 of a G-code file as the reference writes it beside the file
/// (bbs_3mf.cpp `bbl_calc_md5`): 32 capital hexadecimal digits.
pub fn gcode_md5(gcode: &[u8]) -> String {
    let digest = Md5::digest(gcode);
    digest.iter().map(|byte| format!("{byte:02X}")).collect()
}

/// A number as nlohmann's `json::dump` writes a double: the shortest that
/// reads back, with ".0" on a whole one.
fn json_number(value: f64) -> String {
    if !value.is_finite() {
        return "null".into();
    }
    let text = format!("{value}");
    if text.contains(['.', 'e', 'E']) {
        text
    } else {
        format!("{text}.0")
    }
}

fn json_text(text: &str) -> String {
    serde_json::Value::from(text).to_string()
}

fn json_box(values: &[f64; 4]) -> String {
    let numbers: Vec<String> = values.iter().map(|v| json_number(*v)).collect();
    format!("[{}]", numbers.join(","))
}

impl PlateBoxes {
    /// `Metadata/plate_N.json`: one line, keys in order, as nlohmann's
    /// `json::dump` writes `PlateBBoxData::to_json`.
    pub fn json(&self) -> String {
        let objects: Vec<String> = self
            .objects
            .iter()
            .map(|object| {
                format!(
                    r#"{{"area":{},"bbox":{},"id":{},"layer_height":{},"name":{}}}"#,
                    json_number(f64::from(object.area)),
                    json_box(&object.bbox),
                    object.id,
                    json_number(f64::from(object.layer_height)),
                    json_text(&object.name)
                )
            })
            .collect();
        let colours: Vec<String> = self.filament_colours.iter().map(|c| json_text(c)).collect();
        let ids: Vec<String> = self.filament_ids.iter().map(u32::to_string).collect();
        let mut out = format!(r#"{{"bbox_all":{}"#, json_box(&self.all));
        // nlohmann leaves a key out that was never set: an empty list of
        // objects is no `bbox_objects` at all.
        if !objects.is_empty() {
            let _ = write!(out, r#","bbox_objects":[{}]"#, objects.join(","));
        }
        let _ = write!(
            out,
            r#","bed_type":{},"filament_colors":[{}],"filament_ids":[{}],"first_extruder":{},"first_layer_time":{},"is_seq_print":{},"nozzle_diameter":{},"version":2}}"#,
            json_text(&self.bed_type),
            colours.join(","),
            ids.join(","),
            self.first_extruder,
            json_number(f64::from(self.first_layer_time)),
            self.is_seq_print,
            json_number(f64::from(self.nozzle_diameter))
        );
        out
    }
}

/// The reference's version as its slice information heads it
/// (`convert_to_full_version`: "02.04.09.00").
pub(crate) fn full_version() -> String {
    let parts: Vec<String> = REFERENCE_VERSION
        .split('.')
        .map(|part| format!("{part:0>2}"))
        .collect();
    parts.join(".")
}

fn joined<T: ToString>(values: &[T], between: &str) -> String {
    values
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(between)
}

/// `Metadata/slice_info.config` of the plates sliced
/// (`_add_slice_info_config_file_to_archive`).
pub(crate) fn slice_info(plates: &[SlicedPlate<'_>]) -> String {
    let mut out = format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<config>\n  <header>\n",
            "    <header_item key=\"X-BBL-Client-Type\" value=\"slicer\"/>\n",
            "    <header_item key=\"X-BBL-Client-Version\" value=\"{}\"/>\n",
            "  </header>\n"
        ),
        full_version()
    );
    for plate in plates {
        let info = &plate.info;
        let meta = |out: &mut String, key: &str, value: &str| {
            let _ = writeln!(
                out,
                r#"    <metadata key="{key}" value="{}"/>"#,
                escape(value)
            );
        };
        out.push_str("  <plate>\n");
        meta(&mut out, "index", &plate.plate.to_string());
        meta(
            &mut out,
            "extruder_type",
            &joined(&info.extruder_types, " "),
        );
        meta(
            &mut out,
            "nozzle_volume_type",
            &joined(&info.nozzle_volume_types, " "),
        );
        meta(&mut out, "printer_model_id", &info.printer_model_id);
        meta(&mut out, "nozzle_diameters", &info.nozzle_diameters);
        meta(&mut out, "timelapse_type", &info.timelapse_type.to_string());
        meta(&mut out, "prediction", &info.prediction.to_string());
        meta(
            &mut out,
            "weight",
            &info
                .weight
                .map_or_else(String::new, |grams| format!("{grams:.2}")),
        );
        meta(&mut out, "pause_count", "0");
        // `std::to_string` of a float: six places.
        meta(
            &mut out,
            "first_layer_time",
            &format!("{:.6}", info.first_layer_time),
        );
        // A G-code file is never written with toolpaths off the bed.
        meta(&mut out, "outside", "false");
        meta(&mut out, "support_used", &info.support_used.to_string());
        meta(
            &mut out,
            "label_object_enabled",
            &info.label_object_enabled.to_string(),
        );
        meta(
            &mut out,
            "support_material_on_wipe_tower",
            &info.support_material_on_wipe_tower.to_string(),
        );
        meta(&mut out, "enable_filament_dynamic_map", "false");
        meta(
            &mut out,
            "has_filament_switcher",
            &info.has_filament_switcher.to_string(),
        );
        meta(&mut out, "filament_maps", &joined(&info.filament_maps, " "));
        for (identify, name) in &info.objects {
            let _ = writeln!(
                out,
                r#"    <object identify_id="{identify}" name="{}" skipped="false" />"#,
                escape(name)
            );
        }
        for filament in &info.filaments {
            let _ = writeln!(
                out,
                concat!(
                    r#"    <filament id="{}" tray_info_idx="{}" type="{}" color="{}" used_m="{:.2}" used_g="{:.2}" group_id="{}" "#,
                    r#"nozzle_diameter="{:.2}" volume_type="{}" used_for_object="{}" used_for_support="{}" total_load_time="0.00" total_unload_time="0.00"/>"#
                ),
                filament.id,
                escape(&filament.tray_info_idx),
                escape(&filament.kind),
                escape(&filament.colour),
                filament.used_m,
                filament.used_g,
                filament.group_id,
                filament.nozzle_diameter,
                escape(&filament.volume_type),
                filament.used_for_object,
                filament.used_for_support
            );
        }
        for nozzle in &info.nozzles {
            let _ = writeln!(
                out,
                r#"    <nozzle id="{}" extruder_id="{}" nozzle_diameter="{:.1}" volume_type="{}"/>"#,
                nozzle.id,
                nozzle.extruder_id,
                nozzle.diameter,
                escape(&nozzle.volume_type)
            );
        }
        out.push_str("  </plate>\n");
    }
    out.push_str("</config>\n");
    out
}

/// `Metadata/filament_sequence.json`: for every plate of the project, the
/// filaments in the order they are taken up and their nozzles; empty
/// lists for a plate not sliced (`_add_filament_sequence_file_to_archive`).
pub(crate) fn filament_sequence(plate_ids: &[u32], plates: &[SlicedPlate<'_>]) -> String {
    let entries: Vec<String> = plate_ids
        .iter()
        .enumerate()
        .map(|(at, id)| {
            let sequence = plates
                .iter()
                .find(|plate| plate.plate == *id)
                .map(|plate| plate.info.sequence.as_slice())
                .unwrap_or_default();
            // No nozzle grouping here: each change is on nozzle 0, as the
            // reference records it without a grouping result
            // (GCode.cpp `record_filament_change`).
            let nozzles = vec!["0"; sequence.len()].join(",");
            format!(
                r#""plate_{}":{{"nozzle_sequence":[{nozzles}],"optimal_assignment":[],"sequence":[{}]}}"#,
                at + 1,
                joined(sequence, ",")
            )
        })
        .collect();
    format!("{{{}}}", entries.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_is_the_references_spelling() {
        assert_eq!(gcode_md5(b""), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(
            gcode_md5(b"The quick brown fox jumps over the lazy dog"),
            "9E107D9D372BB6826BD81D3542A419D6"
        );
    }

    #[test]
    fn boxes_are_written_as_nlohmann_writes_them() {
        // The reference's own plate_1.json of support_corpus/twopart_groups.3mf.
        let boxes = PlateBoxes {
            all: [134.5, 128.0, 142.5, 144.0],
            objects: vec![ObjectBox {
                id: 44,
                bbox: [134.5, 128.0, 142.5, 144.0],
                area: 128.0,
                layer_height: 0.25,
                name: "assembly".into(),
            }],
            filament_ids: vec![0],
            filament_colours: vec!["#F2754E".into()],
            is_seq_print: false,
            first_extruder: 0,
            nozzle_diameter: 0.4,
            bed_type: "cool_plate".into(),
            first_layer_time: 0.0,
        };
        assert_eq!(
            boxes.json(),
            r##"{"bbox_all":[134.5,128.0,142.5,144.0],"bbox_objects":[{"area":128.0,"bbox":[134.5,128.0,142.5,144.0],"id":44,"layer_height":0.25,"name":"assembly"}],"bed_type":"cool_plate","filament_colors":["#F2754E"],"filament_ids":[0],"first_extruder":0,"first_layer_time":0.0,"is_seq_print":false,"nozzle_diameter":0.4000000059604645,"version":2}"##
        );
    }

    #[test]
    fn sequences_name_every_plate() {
        let plate = SlicedPlate {
            plate: 2,
            info: SliceInfo {
                sequence: vec![2, 6, 4],
                ..SliceInfo::default()
            },
            ..SlicedPlate::default()
        };
        assert_eq!(
            filament_sequence(&[1, 2], &[plate]),
            r#"{"plate_1":{"nozzle_sequence":[],"optimal_assignment":[],"sequence":[]},"plate_2":{"nozzle_sequence":[0,0,0],"optimal_assignment":[],"sequence":[2,6,4]}}"#
        );
    }
}
