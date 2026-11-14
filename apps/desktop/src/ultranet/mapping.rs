//! Which AMS tray feeds each filament of a plate: the reference's
//! auto-match (DeviceManager.cpp `MachineObject::ams_filament_mapping`
//! l.948, for a printer with one extruder), its check
//! (`is_valid_mapping_result` l.1249), the three strings the print job
//! carries (BambuSendMapping.cpp `compose`), the trays' names
//! (GUI_App.cpp `transition_tridid`) and the AMS's list of filaments it
//! does not take (DeviceManager.cpp `check_filaments_in_blacklist`).
//!
//! Where this differs from the reference on purpose (each an error of the
//! reference's, said in the report and UI_PARITY.md):
//!
//! - The colour distance is CIE76 in Lab from sRGB in 0..1. The reference
//!   hands `RGB2Lab` 0..255 values where it expects 0..1
//!   (DeviceManager.cpp l.891, ColorSpaceConvert.cpp l.22), so its
//!   "Lab" is not Lab and some nearest colours differ.
//! - A tray picked keeps the distance it was picked at. The reference
//!   copies the tray's own `distance` (DeviceManager.cpp l.1170), a
//!   `FilamentInfo` field never set, which the last fallback then
//!   compares with 6000.
//! - On a tie the filament's preset id is compared with the tray's own
//!   (`tray_filaments` is a map by tray index, which the reference indexes
//!   by column number, l.1156).
//! - The blacklist's "Third Party" entries are for filaments not Bambu
//!   Lab's: the reference compares the entry's own vendor with
//!   "bambulab" (always true there, l.8691), and compares "Bambu Lab"
//!   (the presets' `filament_vendor`) with "bambulab", which never match.

use serde_json::{Value, json};

use super::machine::Machine;

/// A filament of the plate (`FilamentInfo` as the send dialog makes it).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filament {
    /// The project's filament, from 0.
    pub id: usize,
    /// "#RRGGBB" or "#RRGGBBAA".
    pub color: String,
    pub kind: String,
    /// The filament preset's id ("GFA00").
    pub filament_id: String,
    /// `filament_vendor`.
    pub brand: String,
}

/// What a filament is fed from.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    pub id: usize,
    /// `ams_id * 4 + slot`, -1 none.
    pub tray_id: i64,
    /// The tray's, "RRGGBBAA".
    pub color: String,
    pub kind: String,
    pub filament_id: String,
    pub ams_id: String,
    pub slot_id: String,
    pub distance: f64,
}

impl Mapped {
    fn none(id: usize) -> Self {
        Mapped {
            id,
            tray_id: -1,
            color: String::new(),
            kind: String::new(),
            filament_id: String::new(),
            ams_id: String::new(),
            slot_id: String::new(),
            distance: 99_999.0,
        }
    }
}

/// `transition_tridid`: tray 0 is "A1", 5 is "B2"; none is "-".
pub fn tray_name(tray_id: i64) -> String {
    const LETTERS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    if tray_id < 0 {
        return "-".into();
    }
    let (index, suffix) = if tray_id >= 128 * 4 {
        ((tray_id - 128 * 4) / 4, None)
    } else {
        (tray_id / 4, Some(tray_id % 4 + 1))
    };
    match LETTERS.get(index as usize) {
        Some(letter) => match suffix {
            Some(n) => format!("{}{n}", *letter as char),
            None => (*letter as char).to_string(),
        },
        None => "?".into(),
    }
}

/// A colour as RGBA: "#RRGGBB", "#RRGGBBAA" (a filament) or "RRGGBBAA" (a
/// tray, `AmsTray::decode_color`: anything else is white).
pub fn rgba(text: &str) -> [u8; 4] {
    let hex = text.trim().trim_start_matches('#');
    let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    match hex.len() {
        6 => match (byte(0), byte(2), byte(4)) {
            (Some(r), Some(g), Some(b)) => [r, g, b, 255],
            _ => [255; 4],
        },
        8 => match (byte(0), byte(2), byte(4), byte(6)) {
            (Some(r), Some(g), Some(b), Some(a)) => [r, g, b, a],
            _ => [255; 4],
        },
        _ => [255; 4],
    }
}

/// CIE Lab of an sRGB colour (D65), as ColorSpaceConvert.cpp means it.
fn lab(rgb: [u8; 4]) -> [f64; 3] {
    let linear = |c: u8| {
        let n = f64::from(c) / 255.0;
        100.0
            * if n > 0.04045 {
                ((n + 0.055) / 1.055).powf(2.4)
            } else {
                n / 12.92
            }
    };
    let (r, g, b) = (linear(rgb[0]), linear(rgb[1]), linear(rgb[2]));
    let x = 0.412453 * r + 0.357580 * g + 0.180423 * b;
    let y = 0.212671 * r + 0.715160 * g + 0.072169 * b;
    let z = 0.019334 * r + 0.119193 * g + 0.950227 * b;
    let pivot = |n: f64| {
        if n > 0.008856 {
            n.cbrt()
        } else {
            7.787 * n + 16.0 / 116.0
        }
    };
    let (fx, fy, fz) = (pivot(x / 95.047), pivot(y / 100.0), pivot(z / 108.883));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// `calc_color_distance`: CIE76.
pub fn color_distance(a: &str, b: &str) -> f64 {
    let (p, q) = (lab(rgba(a)), lab(rgba(b)));
    ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2)).sqrt()
}

/// A tray that can be mapped to, with its tray index.
struct Offered {
    tray_id: i64,
    color: String,
    kind: String,
    filament_id: String,
    ams_id: String,
    slot_id: String,
}

/// `ams_filament_mapping` for a printer with one extruder: the nearest
/// colour of the same type among the trays that say what they hold, each
/// tray once (a filament of exactly the same colour and type may share
/// one); failing that, the filaments in tray order; failing that, the
/// colour matches kept. Answers the reference's code: 0 mapped, -1
/// nothing to map.
pub fn auto_map(machine: &Machine, filaments: &[Filament]) -> (i32, Vec<Mapped>) {
    if filaments.is_empty() {
        return (-1, Vec::new());
    }
    // Every tray that says what it holds, by tray index.
    let mut offered: Vec<Offered> = Vec::new();
    for ams in &machine.ams {
        let ams_number: i64 = ams.id.parse().unwrap_or(0);
        for tray in &ams.trays {
            if !tray.ready() {
                continue;
            }
            offered.push(Offered {
                tray_id: ams_number * 4 + tray.id.parse::<i64>().unwrap_or(0),
                color: tray.color.clone(),
                kind: tray.filament_type(),
                filament_id: tray.setting_id.clone(),
                ams_id: ams.id.clone(),
                slot_id: tray.id.clone(),
            });
        }
    }
    offered.sort_by_key(|tray| tray.tray_id);
    // Every tray in order, for the order mapping, numbered as the
    // reference numbers them (one after another).
    let mut in_order: Vec<Offered> = Vec::new();
    for ams in &machine.ams {
        for tray in &ams.trays {
            in_order.push(Offered {
                tray_id: in_order.len() as i64,
                color: tray.color.clone(),
                kind: tray.filament_type(),
                filament_id: tray.setting_id.clone(),
                ams_id: ams.id.clone(),
                slot_id: tray.id.clone(),
            });
        }
    }

    struct Cell {
        distance: f64,
        type_match: bool,
    }
    let distances: Vec<Vec<Cell>> = filaments
        .iter()
        .map(|filament| {
            offered
                .iter()
                .map(|tray| {
                    let (fa, ta) = (rgba(&filament.color)[3], rgba(&tray.color)[3]);
                    if filament.kind != tray.kind {
                        Cell {
                            distance: 999_999.0,
                            type_match: false,
                        }
                    } else {
                        Cell {
                            distance: if fa != ta {
                                999_999.0
                            } else {
                                color_distance(&filament.color, &tray.color)
                            },
                            type_match: true,
                        }
                    }
                })
                .collect()
        })
        .collect();

    let mut result: Vec<Mapped> = filaments.iter().map(|f| Mapped::none(f.id)).collect();
    let mut picked_src = vec![false; filaments.len()];
    let mut picked_tar = vec![false; offered.len()];
    for _ in 0..filaments.len() {
        let mut min = f64::from(i32::MAX);
        let mut pick: Option<(usize, usize)> = None;
        for (i, row) in distances.iter().enumerate() {
            if picked_src[i] {
                continue;
            }
            for (j, cell) in row.iter().enumerate() {
                if picked_tar[j] {
                    // A tray already given may be shared by the same colour.
                    if cell.type_match && cell.distance < 0.0001 {
                        min = cell.distance;
                        pick = Some((i, j));
                    }
                    continue;
                }
                if !cell.type_match {
                    continue;
                }
                if min > cell.distance {
                    min = cell.distance;
                    pick = Some((i, j));
                } else if min == cell.distance
                    && let Some((pi, pj)) = pick
                    && filaments[pi].filament_id != offered[pj].filament_id
                    && filaments[i].filament_id == offered[j].filament_id
                {
                    pick = Some((i, j));
                }
            }
        }
        let Some((i, j)) = pick else { continue };
        let tray = &offered[j];
        result[i] = Mapped {
            id: filaments[i].id,
            tray_id: tray.tray_id,
            color: tray.color.clone(),
            kind: tray.kind.clone(),
            filament_id: tray.filament_id.clone(),
            ams_id: tray.ams_id.clone(),
            slot_id: tray.slot_id.clone(),
            distance: distances[i][j].distance,
        };
        picked_src[i] = true;
        picked_tar[j] = true;
    }

    let by_colour = result.clone();
    if is_valid(machine, &mut result, true) {
        return (0, result);
    }
    // The filaments in tray order.
    let mut in_turn: Vec<Mapped> = filaments.iter().map(|f| Mapped::none(f.id)).collect();
    let mut ordered = true;
    for (i, filament) in filaments.iter().enumerate() {
        let Some(tray) = in_order.get(i) else {
            ordered = false;
            break;
        };
        if !tray.kind.is_empty() && tray.kind != filament.kind {
            ordered = false;
            break;
        }
        in_turn[i] = Mapped {
            id: filament.id,
            tray_id: tray.tray_id,
            color: tray.color.clone(),
            kind: tray.kind.clone(),
            filament_id: tray.filament_id.clone(),
            ams_id: tray.ams_id.clone(),
            slot_id: tray.slot_id.clone(),
            distance: 99_999.0,
        };
    }
    if ordered && is_valid(machine, &mut in_turn, true) {
        return (0, in_turn);
    }
    // The colour matches, the far ones dropped.
    let mut result = by_colour;
    for mapped in &mut result {
        if mapped.distance >= 6000.0 {
            *mapped = Mapped::none(mapped.id);
        }
    }
    (0, result)
}

/// `is_valid_mapping_result`: every filament on a tray of an AMS that is
/// there (and, with `check_empty`, a tray that holds a spool). A filament
/// that is not is set to none.
pub fn is_valid(machine: &Machine, result: &mut [Mapped], check_empty: bool) -> bool {
    if result.is_empty() {
        return false;
    }
    let mut valid = true;
    for mapped in result.iter_mut() {
        if mapped.tray_id < 0 {
            valid = false;
            continue;
        }
        let ams_id = (mapped.tray_id / 4).to_string();
        let Some(ams) = machine.ams.iter().find(|ams| ams.id == ams_id) else {
            mapped.tray_id = -1;
            valid = false;
            continue;
        };
        if check_empty {
            let slot = (mapped.tray_id % 4).to_string();
            if !ams.trays.iter().any(|tray| tray.id == slot && tray.exists) {
                mapped.tray_id = -1;
                valid = false;
            }
        }
    }
    valid
}

/// The tray a person picked for a filament (`-1` none), read from the
/// printer as `MachineObject::ams_filament_mapping` reads a tray.
pub fn picked(machine: &Machine, id: usize, tray_id: i64) -> Mapped {
    let Some(tray) = (tray_id >= 0).then(|| machine.tray(tray_id)).flatten() else {
        return Mapped::none(id);
    };
    Mapped {
        id,
        tray_id,
        color: tray.color.clone(),
        kind: tray.filament_type(),
        filament_id: tray.setting_id.clone(),
        ams_id: (tray_id / 4).to_string(),
        slot_id: tray.id.clone(),
        distance: 0.0,
    }
}

/// The three strings of a print job (`BambuSendMapping::compose`), for a
/// printer with one nozzle: the tray of each of the project's filaments
/// (`ams_mapping`), `[{ams_id, slot_id}]` (`ams_mapping2`), and each
/// filament's colours and types (`ams_mapping_info`). `None` when nothing
/// is mapped. The bool: every filament has a tray.
pub fn compose(
    result: &[Mapped],
    filaments: &[Filament],
    project_count: usize,
    filament_ids: &[String],
) -> Option<(String, String, String, bool)> {
    if result.is_empty() || result.iter().all(|r| r.tray_id == -1) {
        return None;
    }
    let valid = result.iter().all(|r| r.tray_id != -1);
    let mut v0 = Vec::new();
    let mut v1 = Vec::new();
    let mut info = Vec::new();
    for i in 0..project_count {
        let mut tray_id = -1;
        let mut item1 = json!({ "ams_id": 0xff, "slot_id": 0xff });
        let mut item =
            json!({ "ams": -1, "targetColor": "", "filamentId": "", "filamentType": "" });
        for (k, r) in result.iter().enumerate() {
            if r.id != i {
                continue;
            }
            tray_id = r.tray_id;
            item["ams"] = json!(tray_id);
            item["filamentType"] = json!(filaments.get(k).map_or(&r.kind, |f| &f.kind));
            if let Some(id) = filament_ids.get(i) {
                item["filamentId"] = json!(id);
            }
            item["sourceColor"] = json!(filaments.get(k).map_or(&r.color, |f| &f.color));
            item["targetColor"] = json!(r.color);
            item1 = match (r.ams_id.parse::<i64>(), r.slot_id.parse::<i64>()) {
                (Ok(ams), Ok(slot)) if !r.ams_id.is_empty() && !r.slot_id.is_empty() => {
                    json!({ "ams_id": ams, "slot_id": slot })
                }
                _ => json!({ "ams_id": 255, "slot_id": 255 }),
            };
        }
        v0.push(json!(tray_id));
        v1.push(item1);
        info.push(item);
    }
    Some((
        Value::Array(v0).to_string(),
        Value::Array(v1).to_string(),
        Value::Array(info).to_string(),
        valid,
    ))
}

/// One entry of `resources/printers/filaments_blacklist.json`.
#[derive(Clone, Debug, PartialEq)]
pub struct Blacklisted {
    pub vendor: String,
    pub kind: String,
    /// "prohibition" or "warning".
    pub action: String,
    /// The reference's sentence for it (its message id).
    pub said: &'static str,
}

/// The entries of the blacklist file, each with the reference's sentence
/// (`check_filaments_in_blacklist`'s table); an entry it has no sentence
/// for is shown by its own description.
pub fn read_blacklist(text: &str) -> Vec<Blacklisted> {
    let Ok(root) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    root["blacklist"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let said = match entry["description"].as_str()? {
                "TPU: not supported" => "TPU is not supported by AMS.",
                "Bambu PET-CF/PA6-CF: not supported" => "Bambu PET-CF/PA6-CF is not supported by AMS.",
                "PVA: flexible" => {
                    "Damp PVA will become flexible and get stuck inside AMS, please take care to dry it before use."
                }
                "CF/GF: hard and brittle" => {
                    "CF/GF filaments are hard and brittle, it's easy to break or get stuck in AMS, please use with caution."
                }
                _ => return None,
            };
            Some(Blacklisted {
                vendor: entry["vendor"].as_str()?.to_owned(),
                kind: entry["type"].as_str()?.to_owned(),
                action: entry["action"].as_str()?.to_owned(),
                said,
            })
        })
        .collect()
}

/// The first entry a filament of `brand` and `kind` falls under. Vendors
/// are compared without case or spaces ("Bambu Lab" is "Bambulab").
pub fn blacklisted<'a>(
    list: &'a [Blacklisted],
    brand: &str,
    kind: &str,
) -> Option<&'a Blacklisted> {
    let plain = |text: &str| text.to_lowercase().replace(' ', "");
    let (brand, kind) = (plain(brand), kind.to_lowercase());
    list.iter().find(|entry| {
        let vendor = plain(&entry.vendor);
        let ours = if vendor == "thirdparty" {
            brand != "bambulab"
        } else {
            vendor == brand
        };
        ours && entry.kind.to_lowercase() == kind
    })
}
