//! "Storage" for the Elegoo Centauri Carbon 1: its G-code files listed,
//! deleted and started over SDCP, ported from the fork's
//! `Utils/ElegooSdcpFiles.cpp` (commands 258 and 259, the start plan) and
//! the parts of `ElegooCanvas.cpp` its start check reads (`AmsConnectStatus`,
//! command 324's `canvas_list`). The pure half is here; the requests are
//! `ElegooLink`'s (`elegoo.rs`).
//!
//! The request bodies are exactly the reference's (pycentauri's warning,
//! which the reference repeats: an unexpected payload shape can crash the
//! printer's firmware and the print on it): 258 `{"Url":"/local"}`, 259
//! `{"FileList":["/local/<path>"]}`, nothing else. The same facts are
//! unverified on the owner's firmware there as here (the reference logs
//! them the first time; this app logs nothing of a printer's).
//!
//! Where this differs from the reference, on purpose:
//! - The reference starts a stored file over a printer whose
//!   `CurrentStatus` is anything but printing (homing, checking a file,
//!   a self-test), since its state reads "standby" then. Here a start needs
//!   every `CurrentStatus` idle, as this app's "Upload and Print" already
//!   asks (`elegoo.rs`).
//! - The reference reads a list or a delete answer at `Data.Data`, `Data`
//!   or the top level of a frame; here the frame is the one whose
//!   `Data.Cmd` is the command asked (the reference's own match), so the
//!   top level is not read.

use serde_json::{Value, json};

use super::{Options, Stored, moonraker};

/// The internal storage as the file commands name it (`LOCAL_ROOT`).
pub const LOCAL_ROOT: &str = "/local";
pub const FILE_LIST: u32 = 258;
pub const DELETE_FILES: u32 = 259;
pub const CANVAS: u32 = 324;

/// The Centauri Carbon 1, whose files (and CANVAS) speak SDCP
/// (ElegooCanvas.cpp `canvas_capable_model`); another SDCP printer is not
/// sent these commands.
pub fn is_cc1(printer_model: &str) -> bool {
    printer_model
        .trim()
        .eq_ignore_ascii_case("Elegoo Centauri Carbon")
}

/// Command 258's data: exactly `{"Url":"/local"}`.
pub fn list_request() -> Value {
    json!({ "Url": LOCAL_ROOT })
}

/// Command 259's data: exactly `{"FileList":["/local/<path>"]}`.
pub fn delete_request(path: &str) -> Value {
    json!({ "FileList": [format!("{LOCAL_ROOT}/{path}")] })
}

/// A printer path ("/local/a.gcode", "local/a.gcode", "a.gcode") relative
/// to the storage root; `None` for another storage ("/usb/...") or a path
/// that is not valid (`relative_path`).
pub fn relative_path(printer_path: &str) -> Option<String> {
    let path = printer_path.trim();
    for prefix in ["/local/", "local/"] {
        if let Some(rest) = path.strip_prefix(prefix) {
            return moonraker::valid_path(rest).then(|| rest.to_owned());
        }
    }
    if path.starts_with('/') {
        return None;
    }
    moonraker::valid_path(path).then(|| path.to_owned())
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The same file whatever folder either names (`same_file`).
pub fn same_file(a: &str, b: &str) -> bool {
    let a = base_name(a);
    !a.is_empty() && a == base_name(b)
}

/// A number, or a string of digits (`sf_number_of`).
fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64().filter(|n| n.is_finite()),
        Value::String(s)
            if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse().ok()
        }
        _ => None,
    }
}

/// An integer at `key`, as `sf_int_at` reads it.
fn int_at(object: &Value, key: &str) -> Option<i64> {
    match &object[key] {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        _ => None,
    }
}

/// The first of `keys` the object carries, not null.
fn first_of<'a>(object: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .map(|key| &object[*key])
        .find(|value| !value.is_null())
}

/// The `Ack` of an answer frame: `Data.Data.Ack`, then `Data.Ack`
/// (`ack_of`).
pub fn ack_of(frame: &Value) -> Option<i64> {
    int_at(&frame["Data"]["Data"], "Ack")
        .filter(|ack| *ack >= 0)
        .or_else(|| int_at(&frame["Data"], "Ack"))
}

/// The list in an answer to 258: `Data.Data.FileList`, else
/// `Data.FileList` (`parse_list_answer`). `None` when there is none.
pub fn list_of(frame: &Value) -> Option<Vec<Stored>> {
    let list = frame["Data"]["Data"]["FileList"]
        .as_array()
        .or_else(|| frame["Data"]["FileList"].as_array())?;
    let mut files = Vec::new();
    for item in list {
        if !item.is_object() {
            continue;
        }
        let Some(name) = ["name", "Name", "path"]
            .iter()
            .filter_map(|key| item[*key].as_str())
            .find(|name| !name.is_empty())
        else {
            continue;
        };
        // The specification: type 0 a folder, 1 a file; the root only.
        if int_at(item, "type") == Some(0) {
            continue;
        }
        let Some(path) = relative_path(name) else {
            continue;
        };
        if path.split('/').any(|part| part.starts_with('.')) || !moonraker::listed(&path) {
            continue;
        }
        let size = first_of(item, &["FileSize", "size", "Size"])
            .and_then(number)
            .filter(|n| *n > 0.0)
            .map(|n| n as u64);
        let layers = first_of(item, &["TotalLayers", "layer", "Layers"])
            .and_then(number)
            .filter(|n| *n > 0.0 && *n < 1e7)
            .map(|n| n as u32);
        // Unix seconds are about 1.7e9 now; milliseconds about 1.7e12.
        let modified = first_of(item, &["CreateTime", "create_time", "date"])
            .and_then(number)
            .filter(|n| *n > 0.0)
            .map(|n| if n > 1e11 { (n / 1000.0).floor() } else { n });
        files.push(Stored {
            printable: moonraker::printable(&path),
            name: path,
            size,
            modified,
            layer_count: layers,
            ..Stored::default()
        });
    }
    // Newest first, the tab's first order.
    files.sort_by(|a, b| {
        b.modified
            .partial_cmp(&a.modified)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Some(files)
}

/// The paths an answer to 259 says were not deleted (`ErrData`).
pub fn not_deleted(frame: &Value) -> Vec<String> {
    frame["Data"]["Data"]["ErrData"]
        .as_array()
        .or_else(|| frame["Data"]["ErrData"].as_array())
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|path| relative_path(path).unwrap_or_else(|| path.to_owned()))
        .collect()
}

/// Command 128's `Ack` as one sentence (`start_ack_text`).
pub fn start_ack_text(ack: i64) -> String {
    match ack {
        1 => "The printer is busy and did not start the print.".into(),
        2 => "The printer could not find the file.".into(),
        3 => "The printer says the file is corrupted (MD5 check failed).".into(),
        4 => "The printer could not read the file.".into(),
        5 => "The printer says the file's resolution does not match.".into(),
        6 => "The printer does not recognise the file format.".into(),
        7 => "The file was sliced for another printer model.".into(),
        other => format!("The printer refused to start the print (Ack {other})."),
    }
}

/// `AmsConnectStatus` anywhere in a status frame, four levels down at
/// most: `Some(true)` attached, `Some(false)` not, `None` when it does not
/// say (`cv_find_ams`).
pub fn canvas_attached(frame: &Value) -> Option<bool> {
    fn find(value: &Value, depth: u32) -> Option<bool> {
        let object = value.as_object().filter(|_| depth <= 4)?;
        if let Some(status) = object
            .get("AmsConnectStatus")
            .and_then(|v| match v {
                Value::Number(n) => n.as_i64(),
                Value::Bool(b) => Some(i64::from(*b)),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            })
            .filter(|n| *n >= 0)
        {
            return Some(status != 0);
        }
        object.values().find_map(|v| find(v, depth + 1))
    }
    find(frame, 0)
}

/// A colour as the CANVAS reports it ("#RRGGBB", "0xRRGGBB", "RRGGBBAA")
/// as upper-case RRGGBBAA; "00000000" when it is not one
/// (`canvas_normalize_color`).
fn normalized_color(color: &str) -> String {
    let value = color.trim();
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .unwrap_or(value);
    let value = value.strip_prefix('#').unwrap_or(value);
    let mut digits: String = value
        .chars()
        .filter(char::is_ascii_hexdigit)
        .map(|c| c.to_ascii_uppercase())
        .collect();
    if digits.len() == 6 {
        digits.push_str("FF");
    }
    if digits.len() == 8 {
        digits
    } else {
        "00000000".into()
    }
}

/// Whether an answer to 324 has a CANVAS with a loaded tray: `None` when
/// it carries no usable `canvas_list` (`canvas_parse_payload`: a tray is
/// loaded when the printer names a material and a colour, of a unit that
/// is connected).
pub fn canvas_loaded(frame: &Value) -> Option<bool> {
    let holder = [&frame["Data"]["Data"], &frame["Data"], frame]
        .into_iter()
        .find(|v| v.get("canvas_list").is_some())?;
    let units = holder["canvas_list"].as_array().filter(|l| !l.is_empty())?;
    Some(units.iter().any(|unit| {
        int_at(unit, "connected").unwrap_or(0) != 0
            && unit["tray_list"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|tray| {
                    let kind = tray["filament_type"].as_str().unwrap_or_default();
                    let color = tray["filament_color"].as_str().unwrap_or_default();
                    !kind.is_empty() && !color.is_empty() && normalized_color(color) != "00000000"
                })
    }))
}

/// What a start from "Storage" will do (`plan_start`): the advice the
/// person reads before it (English, as the reference's), and whether the
/// command carries an empty `slot_map` (a CANVAS answered).
#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    pub advice: String,
    pub canvas: bool,
}

/// `canvas`: `None` no CANVAS answered, `Some(loaded)` one did.
pub fn plan(options: &Options, canvas: Option<bool>) -> Plan {
    let on = |flag: bool| if flag { "on" } else { "off" };
    let mut advice = format!(
        "Bed leveling before printing: {}. Timelapse: {}. Plate: {}. (The choices of your last Elegoo send; change them in the send dialog.)",
        on(options.leveling),
        on(options.timelapse),
        if options.smooth_plate {
            "smooth PC side"
        } else {
            "textured PTE side"
        }
    );
    if canvas == Some(true) {
        advice.push_str("\n\nA CANVAS with loaded trays is attached. EdgeSlicer cannot tell which filaments this file uses, so no tray mapping is sent: the printer feeds from its own choice of trays, as when every filament is left \"As sliced\" in the send dialog. Make sure the trays hold the filaments the file was sliced for.");
    }
    Plan {
        advice,
        canvas: canvas.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_read_as_the_reference_reads_them() {
        assert_eq!(relative_path("/local/a.gcode").as_deref(), Some("a.gcode"));
        assert_eq!(relative_path("local/a.gcode").as_deref(), Some("a.gcode"));
        assert_eq!(relative_path(" a.gcode ").as_deref(), Some("a.gcode"));
        assert_eq!(relative_path("/usb/a.gcode"), None);
        assert_eq!(relative_path("/local/../x.gcode"), None);
        assert!(same_file("/local/a.gcode", "a.gcode"));
        assert!(!same_file("", ""));
        assert_eq!(
            delete_request("a b.gcode"),
            json!({ "FileList": ["/local/a b.gcode"] })
        );
        assert_eq!(list_request().to_string(), r#"{"Url":"/local"}"#);
    }

    #[test]
    fn a_list_is_read_with_its_sizes_layers_and_times() {
        let frame = json!({ "Data": { "Cmd": 258, "Data": { "Ack": 0, "FileList": [
            { "name": "/local/boat.gcode", "FileSize": 1000, "TotalLayers": 120, "CreateTime": 1_700_000_000 },
            { "name": "/local/new.gcode", "FileSize": "2000", "CreateTime": 1_700_000_100_000u64 },
            { "name": "/local/folder", "type": 0 },
            { "name": "/usb/stick.gcode" },
            { "name": "/local/.hidden.gcode" },
            { "name": "/local/notes.txt" },
            { "name": "/local/model.3mf" },
        ] } } });
        let files = list_of(&frame).unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["new.gcode", "boat.gcode", "model.3mf"]);
        assert_eq!(files[0].modified, Some(1_700_000_100.0));
        assert_eq!(files[0].size, Some(2000));
        assert_eq!(files[1].layer_count, Some(120));
        assert!(!files[2].printable);
        assert_eq!(list_of(&json!({ "Data": { "Data": { "Ack": 0 } } })), None);
        assert_eq!(
            ack_of(&json!({ "Data": { "Data": { "Ack": 2 } } })),
            Some(2)
        );
        assert_eq!(
            not_deleted(&json!({ "Data": { "Data": { "ErrData": ["/local/a.gcode"] } } })),
            ["a.gcode"]
        );
    }

    #[test]
    fn the_canvas_is_read_as_the_reference_reads_it() {
        assert_eq!(
            canvas_attached(&json!({ "Status": { "AmsConnectStatus": 1 } })),
            Some(true)
        );
        assert_eq!(
            canvas_attached(&json!({ "Status": { "AmsConnectStatus": 0 } })),
            Some(false)
        );
        assert_eq!(canvas_attached(&json!({ "Status": {} })), None);
        let loaded = json!({ "Data": { "Data": { "canvas_list": [
            { "canvas_id": 0, "connected": 1, "tray_list": [
                { "tray_id": 0, "filament_type": "PLA", "filament_color": "#FF0000" },
            ] },
        ] } } });
        assert_eq!(canvas_loaded(&loaded), Some(true));
        // Black is a colour: "#000000" is "000000FF".
        assert_eq!(normalized_color("#000000"), "000000FF");
        let empty = json!({ "Data": { "Data": { "canvas_list": [
            { "canvas_id": 0, "connected": 1, "tray_list": [
                { "tray_id": 0, "filament_type": "", "filament_color": "" },
                { "tray_id": 1, "filament_type": "PLA", "filament_color": "00000000" },
                { "tray_id": 2, "filament_type": "PLA", "filament_color": "nonsense" },
            ] },
        ] } } });
        assert_eq!(canvas_loaded(&empty), Some(false));
        assert_eq!(canvas_loaded(&json!({ "Data": { "Data": {} } })), None);
        let advice = plan(&Options::default(), Some(true));
        assert!(advice.canvas && advice.advice.contains("A CANVAS with loaded trays"));
        let none = plan(
            &Options {
                leveling: true,
                ..Options::default()
            },
            None,
        );
        assert!(!none.canvas);
        assert!(none.advice.starts_with(
            "Bed leveling before printing: on. Timelapse: off. Plate: textured PTE side."
        ));
    }
}
