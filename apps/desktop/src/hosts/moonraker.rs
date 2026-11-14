//! What a Moonraker printer says of itself, read the way the reference
//! reads it: the toolheads and what is loaded in them (SnapmakerLan.cpp
//! `toolheads_of`, `color_of`), the state, and how far a print has got
//! (Utils/MoonrakerProgress.cpp, which works the percent out as the U1's
//! own device page does). Only questions; nothing here tells a printer to
//! do anything.

use serde_json::Value;

use super::{Http, Status, Stored, Toolhead, http_error, snapmaker::encode};

/// Snapmaker's tool changers have four toolheads (SnapmakerTaskConfig.hpp
/// `TOOLHEAD_COUNT`); the query asks for that many extruders.
pub const QUERY_TOOLHEADS: &str = "extruder&extruder1&extruder2&extruder3";

/// `printer/objects/query?` for the toolheads.
pub fn toolheads_query() -> String {
    format!("printer/objects/query?print_stats&print_task_config&{QUERY_TOOLHEADS}")
}

/// `printer/objects/query?` for the state (SnapmakerLan.cpp l.803, the
/// controls left out: nothing is set from here).
pub fn status_query() -> String {
    format!(
        "printer/objects/query?print_stats&display_status&virtual_sdcard&heater_bed&{QUERY_TOOLHEADS}"
    )
}

fn extruder_key(index: usize) -> String {
    if index == 0 {
        "extruder".into()
    } else {
        format!("extruder{index}")
    }
}

fn text_at(list: &Value, i: usize) -> String {
    list.get(i)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn flag_at(list: &Value, i: usize) -> bool {
    list.get(i).and_then(Value::as_bool).unwrap_or(false)
}

fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite())
}

/// `#RRGGBB` of toolhead `i`: `filament_color_rgba` (RRGGBBAA), else the
/// ARGB number of `filament_color`.
fn color_of(config: &Value, i: usize) -> String {
    if let Some(rgba) = config["filament_color_rgba"].get(i).and_then(Value::as_str)
        && rgba.len() >= 6
        && rgba.is_char_boundary(6)
    {
        return format!("#{}", &rgba[..6]);
    }
    if let Some(argb) = config["filament_color"].get(i).and_then(Value::as_i64) {
        return format!("#{:06X}", argb & 0xFF_FFFF);
    }
    String::new()
}

/// The toolheads of a query's `status` object (`result.status`).
pub fn toolheads_of(status: &Value) -> Vec<Toolhead> {
    let config = &status["print_task_config"];
    if !config.is_object() {
        return Vec::new();
    }
    let count = ["filament_type", "filament_color_rgba", "filament_exist"]
        .iter()
        .filter_map(|key| config[*key].as_array().map(Vec::len))
        .max()
        .unwrap_or(0);
    (0..count)
        .map(|i| Toolhead {
            index: i,
            filament_type: text_at(&config["filament_type"], i),
            sub_type: text_at(&config["filament_sub_type"], i),
            vendor: text_at(&config["filament_vendor"], i),
            color: color_of(config, i),
            loaded: flag_at(&config["filament_exist"], i),
            official: flag_at(&config["filament_official"], i),
            nozzle: number(&status[extruder_key(i)]["nozzle_diameter"]).unwrap_or(0.0),
        })
        .collect()
}

/// The file the printer is on: `print_stats.filename`, else what follows
/// `gcodes/` in `virtual_sdcard.file_path` (MoonrakerProgress.cpp
/// `metadata_filename`).
pub fn printing_file(status: &Value) -> String {
    let name = status["print_stats"]["filename"]
        .as_str()
        .unwrap_or_default();
    if !name.is_empty() {
        return name.to_owned();
    }
    let path = status["virtual_sdcard"]["file_path"]
        .as_str()
        .unwrap_or_default();
    path.split_once("gcodes/")
        .map(|(_, rest)| rest.to_owned())
        .unwrap_or_default()
}

/// The printer's state with the percent and the time left worked out as
/// MoonrakerProgress.cpp `estimate` does: the share of the G-code body
/// read (`file_position` against the file's `gcode_start_byte` and
/// `gcode_end_byte`), else `display_status.progress`, else
/// `virtual_sdcard.progress`; the time left the slicer's estimate scaled
/// by it, else a line from the time so far once 5 % is done.
/// `metadata` is `server/files/metadata`'s `result` for the file, if asked.
pub fn status_of(status: &Value, metadata: Option<&Value>) -> Status {
    let stats = &status["print_stats"];
    let state = stats["state"].as_str().unwrap_or_default().to_owned();
    let card = &status["virtual_sdcard"];
    let position = number(&card["file_position"]).filter(|n| *n >= 0.0);
    let body = metadata.and_then(|meta| {
        let start = number(&meta["gcode_start_byte"])?;
        let end = number(&meta["gcode_end_byte"])?;
        (start >= 0.0 && end > start).then_some((start, end))
    });
    let fraction = match (body, position) {
        (Some((start, end)), Some(at)) => {
            if at > end {
                1.0
            } else if at < start {
                0.0
            } else {
                (at - start) / (end - start)
            }
        }
        _ => number(&status["display_status"]["progress"])
            .or_else(|| number(&card["progress"]))
            .unwrap_or(0.0),
    }
    .clamp(0.0, 1.0);
    let on_job = state == "printing" || state == "paused";
    let estimated = metadata
        .and_then(|meta| number(&meta["estimated_time"]))
        .filter(|t| *t > 0.0);
    let spent = number(&stats["print_duration"]).filter(|t| *t > 0.0);
    let left = if !on_job {
        0.0
    } else if let Some(estimated) = estimated {
        (estimated * (1.0 - fraction)).max(0.0)
    } else if let Some(spent) = spent.filter(|_| (0.05..1.0).contains(&fraction)) {
        spent / fraction - spent
    } else {
        0.0
    };
    let info = &stats["info"];
    let pair = |heater: &Value| {
        Some((
            number(&heater["temperature"])?,
            number(&heater["target"]).unwrap_or(0.0),
        ))
    };
    let nozzles = (0..4)
        .map(|i| pair(&status[extruder_key(i)]))
        .take_while(Option::is_some)
        .flatten()
        .collect();
    Status {
        state,
        filename: printing_file(status),
        message: stats["message"].as_str().unwrap_or_default().to_owned(),
        percent: (fraction * 100.0).round() as u32,
        left: left.round() as u32,
        layer: number(&info["current_layer"]).unwrap_or(0.0) as u32,
        total_layers: number(&info["total_layer"]).unwrap_or(0.0) as u32,
        bed: pair(&status["heater_bed"]),
        nozzles,
    }
}

// ---- the files on the printer (Utils/PrinterStorage.cpp) ----

/// A short GET answered with Moonraker's `result`.
pub fn result(http: &Http, path: &str) -> Result<Value, String> {
    let answer = http.get(path)?;
    if !answer.ok() {
        return Err(http_error(&answer));
    }
    answer
        .json()
        .and_then(|json| json.get("result").cloned())
        .ok_or_else(|| "The printer's answer is not Moonraker's.".to_owned())
}

fn ends_with(name: &str, extensions: &[&str]) -> bool {
    let lower = name.to_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

/// `is_printable_name`.
pub fn printable(name: &str) -> bool {
    ends_with(name, &[".gcode", ".g", ".gco"])
}

/// What the Storage tab lists: printable names, `.bgcode` and `.3mf`, not
/// under a hidden folder (`.thumbs/`), with a path it may hand back
/// (`parse_file_list`, `valid_path`, `hidden_path`).
pub fn files_of(list: &Value) -> Vec<Stored> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let path = item["path"]
                .as_str()
                .or_else(|| item["filename"].as_str())?
                .trim_start_matches('/');
            let valid = !path.is_empty()
                && path.len() <= 1024
                && !path.ends_with('/')
                && !path.contains('\\')
                && !path.chars().any(char::is_control)
                && path
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..");
            let hidden = path.split('/').any(|part| part.starts_with('.'));
            let listed = printable(path) || ends_with(path, &[".bgcode", ".3mf"]);
            (valid && !hidden && listed).then(|| Stored {
                name: path.to_owned(),
                size: item["size"].as_f64().filter(|n| *n > 0.0).map(|n| n as u64),
                modified: item["modified"].as_f64(),
                printable: printable(path),
                print_time: None,
                filament_mm: None,
                filament_g: None,
                filament_type: String::new(),
            })
        })
        .collect()
}

/// The printer's files with what the slicer wrote into each (the
/// reference asks `server/files/metadata` for each card as it is shown;
/// here for the first hundred).
pub fn list_files(http: &Http) -> Result<Vec<Stored>, String> {
    let mut files = files_of(&result(http, "server/files/list?root=gcodes")?);
    for file in files.iter_mut().take(100) {
        let Ok(meta) = result(
            http,
            &format!(
                "server/files/metadata?filename={}",
                encode(&file.name, false)
            ),
        ) else {
            continue;
        };
        let positive = |key: &str| meta[key].as_f64().filter(|n| *n > 0.0);
        file.print_time = positive("estimated_time").map(|t| t.round() as u32);
        file.filament_mm = positive("filament_total");
        file.filament_g = positive("filament_weight_total");
        file.filament_type = meta["filament_type"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
    }
    // Newest first, the reference's first order.
    files.sort_by(|a, b| {
        b.modified
            .partial_cmp(&a.modified)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(files)
}

/// `DELETE server/files/gcodes/<path>`.
pub fn delete_file(http: &Http, name: &str) -> Result<(), String> {
    let answer = http.delete(&format!("server/files/gcodes/{}", encode(name, true)))?;
    if answer.ok() {
        Ok(())
    } else {
        Err(http_error(&answer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn toolheads_are_read_from_print_task_config() {
        let status = json!({
            "print_task_config": {
                "filament_type": ["PLA", "PETG", "", "PLA"],
                "filament_sub_type": ["Matte", "", "", ""],
                "filament_vendor": ["Snapmaker", "Generic", "", ""],
                "filament_color_rgba": ["FF0000FF", "00ff00ff", "", ""],
                "filament_color": [0, 0, 0, 0xFF_123456_i64],
                "filament_exist": [true, true, false, true],
                "filament_official": [true, false, false, false],
            },
            "extruder": { "nozzle_diameter": 0.4 },
            "extruder1": { "nozzle_diameter": 0.6 },
        });
        let heads = toolheads_of(&status);
        assert_eq!(heads.len(), 4);
        assert_eq!(heads[0].color, "#FF0000");
        assert_eq!(heads[1].color, "#00ff00");
        assert_eq!(heads[3].color, "#123456");
        assert!(heads[0].loaded && !heads[2].loaded && heads[0].official);
        assert_eq!(
            (heads[0].nozzle, heads[1].nozzle, heads[2].nozzle),
            (0.4, 0.6, 0.0)
        );
        assert_eq!(heads[0].sub_type, "Matte");
        assert!(toolheads_of(&json!({})).is_empty());
    }

    #[test]
    fn progress_is_the_share_of_the_body_as_the_device_page_has_it() {
        let status = json!({
            "print_stats": { "state": "printing", "filename": "cube.gcode", "print_duration": 600.0,
                             "info": { "current_layer": 11, "total_layer": 322 } },
            "display_status": { "progress": 0.08 },
            "virtual_sdcard": { "file_position": 1300, "progress": 0.13 },
            "heater_bed": { "temperature": 60.2, "target": 60.0 },
            "extruder": { "temperature": 220.0, "target": 220.0 },
            "extruder1": { "temperature": 25.0, "target": 0.0 },
        });
        let meta =
            json!({ "gcode_start_byte": 1000, "gcode_end_byte": 11000, "estimated_time": 4000.0 });
        let with = status_of(&status, Some(&meta));
        assert_eq!(with.percent, 3);
        assert_eq!(with.left, 3880);
        assert_eq!((with.layer, with.total_layers), (11, 322));
        assert_eq!(with.bed, Some((60.2, 60.0)));
        assert_eq!(with.nozzles.len(), 2);
        // Without the file's facts: display_status, and no line before 5 %.
        let without = status_of(&status, None);
        assert_eq!((without.percent, without.left), (8, 6900));
        // Past the body: 100 %; idle: no time left.
        let done = json!({ "print_stats": { "state": "complete" }, "virtual_sdcard": { "file_position": 20000 } });
        let finished = status_of(&done, Some(&meta));
        assert_eq!((finished.percent, finished.left), (100, 0));
        let early = json!({ "print_stats": { "state": "printing", "print_duration": 50.0 }, "virtual_sdcard": { "progress": 0.02 } });
        assert_eq!(status_of(&early, None).left, 0);
        let path = json!({ "virtual_sdcard": { "file_path": "/home/lava/printer_data/gcodes/sub/a.gcode" } });
        assert_eq!(printing_file(&path), "sub/a.gcode");
    }
}
