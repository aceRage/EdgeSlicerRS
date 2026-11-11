//! What the person set at layers of a plate (pauses, custom G-code, the
//! printer's template, filament changes) turned into what the print does:
//! the filament the plate is printed in from a layer up, and the G-code
//! written at a layer.
//!
//! Ported from ToolOrdering.cpp (the `per_layer_extruder_switches` of the
//! constructor, and `assign_custom_gcodes`), CustomGCode.cpp
//! (`custom_tool_changes`) and GCode.cpp
//! (`ProcessLayer::emit_custom_gcode_per_print_z`).

use edge_3mf::{CustomGcode, CustomItem, CustomKind, CustomMode};
use edge_config::Settings;
use edge_gcode::{AtLayer, LayerCode};

use crate::Failure;

/// libslic3r's `EPSILON`.
const EPSILON: f64 = 1e-4;

/// How many filaments the settings have (`filament_diameter`), as the
/// reference counts them for this.
fn filament_count(settings: &Settings) -> usize {
    settings
        .texts("filament_diameter")
        .map_or(1, <[String]>::len)
        .max(1)
}

/// What an item is, in the slider's words.
fn named(item: &CustomItem) -> String {
    let what = match item.kind {
        CustomKind::PausePrint => "Pause".to_owned(),
        CustomKind::Template => "Custom Template".to_owned(),
        CustomKind::Custom | CustomKind::Unknown => "Custom G-code".to_owned(),
        CustomKind::ToolChange => format!("Change Filament (filament {})", item.extruder),
        CustomKind::ColorChange => "Color change".to_owned(),
    };
    format!("{what} at {} mm", round_mm(item.print_z))
}

fn round_mm(z: f64) -> String {
    let text = format!("{z:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The filament changes a plate's items make: from each height up, the
/// plate prints in that filament (from 1; 0 is back in its own). As the
/// reference: only with more than one filament, a plate printed in one
/// of them, and items made for such a plate (`MultiAsSingle`); a filament
/// past the last is filament 1. `object_filaments` is what the plate
/// prints in without them. What is not done is said.
pub(crate) fn switches(
    info: Option<&CustomGcode>,
    settings: &Settings,
    object_filaments: &[usize],
) -> (Vec<(f64, usize)>, Vec<String>) {
    let Some(info) = info else {
        return (Vec::new(), Vec::new());
    };
    let changes: Vec<&CustomItem> = info
        .items
        .iter()
        .filter(|item| item.kind == CustomKind::ToolChange)
        .collect();
    if changes.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let count = filament_count(settings);
    let mut notes = Vec::new();
    if count > 1 && object_filaments.len() == 1 && info.mode() == CustomMode::MultiAsSingle {
        let mut out: Vec<(f64, usize)> = changes
            .iter()
            .map(|item| {
                let filament = usize::try_from(item.extruder).unwrap_or(0);
                (item.print_z, if filament > count { 1 } else { filament })
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        return (out, notes);
    }
    // Nor anywhere else: `assign_custom_gcodes` passes every filament
    // change by, so the colour change it would make of one on a printer of
    // one filament is never made either.
    let why = if count == 1 {
        "the project has one filament"
    } else if object_filaments.len() > 1 {
        "the plate prints in more than one filament"
    } else {
        "it was set for a plate printed in several filaments together"
    };
    for item in changes {
        notes.push(format!(
            "{} is not done: {why}, and the reference leaves it out too",
            named(item)
        ));
    }
    (Vec::new(), notes)
}

/// Every layer's filament after the changes: `own` is the plate's one
/// filament, `tops` each layer's top. As the reference: a change applies
/// from the first layer whose top is above its height, less `EPSILON`.
pub(crate) fn filament_by_layer(switches: &[(f64, usize)], own: usize, tops: &[f64]) -> Vec<usize> {
    let mut at = 0;
    let mut current = 0;
    tops.iter()
        .map(|top| {
            while at < switches.len() && switches[at].0 < top + EPSILON {
                current = switches[at].1;
                at += 1;
            }
            if current == 0 { own } else { current }
        })
        .collect()
}

/// What is written at each layer. `layers` is each printed layer's top and
/// the filaments printing on it (from 0), in order of height;
/// `object_filaments` what the plate prints in before any change. Each
/// layer takes the item nearest it, the highest when several are; a
/// filament change is not written here (it is made by the layers'
/// filaments) but on a printer of one filament, where the reference makes
/// it a colour change. What is not written is said.
pub(crate) fn assign(
    info: Option<&CustomGcode>,
    settings: &Settings,
    layers: &[(f64, Vec<usize>)],
    object_filaments: &[usize],
) -> Result<(Vec<AtLayer>, Vec<String>), Failure> {
    let Some(info) = info.filter(|info| !info.items.is_empty()) else {
        return Ok((Vec::new(), Vec::new()));
    };
    let count = filament_count(settings);
    let mode = if count == 1 {
        CustomMode::SingleExtruder
    } else if object_filaments.len() == 1 {
        CustomMode::MultiAsSingle
    } else {
        CustomMode::MultiExtruder
    };
    let made_for = info.mode();
    // Tool and colour changes made for printing several filaments together
    // are not done on a print of one, nor the other way round.
    let ignore_changes =
        (mode == CustomMode::MultiExtruder) != (made_for == CustomMode::MultiExtruder);
    // On a printer of one filament a filament change is a colour change.
    let changes_as_colour =
        mode == CustomMode::SingleExtruder && made_for == CustomMode::MultiAsSingle;

    let mut items: Vec<&CustomItem> = info.items.iter().collect();
    items.sort_by(|a, b| a.print_z.total_cmp(&b.print_z));
    let mut notes = Vec::new();
    let mut chosen: Vec<(usize, &CustomItem)> = Vec::new();
    let mut printing_above = vec![false; count];
    let mut left = items.len();
    let mut z_above = f64::MIN;
    for at in (0..layers.len()).rev() {
        let (z, filaments) = (&layers[at].0, &layers[at].1);
        for filament in filaments {
            if let Some(above) = printing_above.get_mut(*filament) {
                *above = true;
            }
        }
        // Items above this layer that a layer above did not take: another
        // was nearer that layer.
        while left > 0
            && ((z_above > *z && items[left - 1].print_z > 0.5 * (z + z_above))
                || items[left - 1].kind == CustomKind::ToolChange)
        {
            let item = items[left - 1];
            if item.kind != CustomKind::ToolChange {
                notes.push(format!(
                    "{} is not written: another item is set at the same layer, and the \
                     reference writes only the highest",
                    named(item)
                ));
            }
            left -= 1;
        }
        z_above = *z;
        if left == 0 {
            break;
        }
        let item = items[left - 1];
        let below = if at > 0 { layers[at - 1].0 } else { 0.0 };
        if item.print_z > 0.5 * (below + z) {
            let colour = item.kind == CustomKind::ColorChange;
            let tool = item.kind == CustomKind::ToolChange;
            let applied = !colour && !tool
                || (!ignore_changes
                    && if colour {
                        mode == CustomMode::SingleExtruder
                            || usize::try_from(item.extruder - 1)
                                .ok()
                                .and_then(|filament| printing_above.get(filament))
                                .copied()
                                .unwrap_or(false)
                    } else {
                        changes_as_colour
                    });
            if applied {
                chosen.push((at, item));
            } else {
                notes.push(format!(
                    "{} is not written: it was set for another kind of print, and the reference \
                     leaves it out too",
                    named(item)
                ));
            }
            left -= 1;
        }
    }
    for item in &items[..left] {
        if item.kind != CustomKind::ToolChange {
            notes.push(format!(
                "{} is not written: no layer of the print is there",
                named(item)
            ));
        }
    }
    chosen.reverse();

    let pause = edge_gcode::gcode_template(settings, "machine_pause_gcode");
    let template = edge_gcode::gcode_template(settings, "template_custom_gcode");
    let mut codes = Vec::with_capacity(chosen.len());
    for (at, item) in chosen {
        let code = match item.kind {
            CustomKind::PausePrint => {
                if pause.is_none() {
                    notes.push(format!(
                        "{}: the printer has no pause G-code (machine_pause_gcode), so the print \
                         does not stop there",
                        named(item)
                    ));
                }
                LayerCode::Pause(pause.clone().unwrap_or_default())
            }
            CustomKind::Template => {
                if template.is_none() {
                    notes.push(format!(
                        "{}: the printer has no template (template_custom_gcode), so nothing is \
                         written there",
                        named(item)
                    ));
                }
                LayerCode::Template(template.clone().unwrap_or_default())
            }
            CustomKind::Custom | CustomKind::Unknown => LayerCode::Custom(item.extra.clone()),
            // What the reference writes for a colour change, and for a
            // filament change on a printer of one filament: the tag and
            // the item's text, which it leaves empty (the `M600` it once
            // wrote is left out, GCode.cpp `#if 0`).
            CustomKind::ColorChange | CustomKind::ToolChange => {
                notes.push(format!(
                    "{} is not done: the reference writes no colour change, only its mark",
                    named(item)
                ));
                LayerCode::Custom(item.extra.clone())
            }
        };
        codes.push(AtLayer {
            z: layers[at].0,
            code,
        });
    }
    Ok((codes, notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(z: f64, kind: CustomKind, extruder: i32, extra: &str) -> CustomItem {
        CustomItem {
            print_z: z,
            kind,
            extruder,
            color: String::new(),
            extra: extra.into(),
            gcode: String::new(),
        }
    }

    fn settings(filaments: usize) -> Settings {
        let list: Vec<String> = vec!["\"1.75\"".into(); filaments];
        Settings::from_json(&format!(
            r#"{{"filament_diameter": [{}], "machine_pause_gcode": "M601", "template_custom_gcode": "M117 [layer_num]"}}"#,
            list.join(", ")
        ))
        .unwrap()
    }

    #[test]
    fn each_item_goes_to_the_layer_nearest_it() {
        let layers: Vec<(f64, Vec<usize>)> =
            [0.2, 0.4, 0.6, 0.8].iter().map(|z| (*z, vec![0])).collect();
        let info = CustomGcode {
            mode: Some(CustomMode::SingleExtruder),
            items: vec![
                item(0.4, CustomKind::PausePrint, 1, ""),
                item(0.61, CustomKind::Custom, 1, "M117 hi"),
                // Two at the top layer: the highest is written.
                item(0.8, CustomKind::Template, 1, ""),
                item(5.0, CustomKind::Custom, 1, "G4 S1"),
                // Below the first layer's middle: nothing.
                item(0.05, CustomKind::Custom, 1, "never"),
            ],
        };
        let (codes, notes) = assign(Some(&info), &settings(1), &layers, &[1]).unwrap();
        assert_eq!(
            codes,
            vec![
                AtLayer {
                    z: 0.4,
                    code: LayerCode::Pause("M601".into())
                },
                AtLayer {
                    z: 0.6,
                    code: LayerCode::Custom("M117 hi".into())
                },
                AtLayer {
                    z: 0.8,
                    code: LayerCode::Custom("G4 S1".into())
                },
            ]
        );
        assert_eq!(notes.len(), 2, "{notes:?}");
        assert!(notes[0].starts_with("Custom Template at 0.8 mm is not written"));
        assert!(notes[1].starts_with("Custom G-code at 0.05 mm is not written"));
    }

    #[test]
    fn a_filament_change_switches_the_plate_from_its_layer_up() {
        let info = CustomGcode {
            mode: Some(CustomMode::MultiAsSingle),
            items: vec![
                item(0.6, CustomKind::ToolChange, 2, ""),
                item(1.0, CustomKind::ToolChange, 9, ""),
            ],
        };
        let (switched, notes) = switches(Some(&info), &settings(2), &[1]);
        assert!(notes.is_empty());
        assert_eq!(switched, vec![(0.6, 2), (1.0, 1)]);
        let tops = [0.2, 0.4, 0.6, 0.8, 1.0, 1.2];
        assert_eq!(
            filament_by_layer(&switched, 1, &tops),
            vec![1, 1, 2, 2, 1, 1]
        );
        // Nothing is written at the layer for it.
        let layers: Vec<(f64, Vec<usize>)> = tops.iter().map(|z| (*z, vec![0])).collect();
        let (codes, notes) = assign(Some(&info), &settings(2), &layers, &[1]).unwrap();
        assert!(codes.is_empty() && notes.is_empty());

        // Not with a plate in two filaments, nor with one filament.
        let (switched, notes) = switches(Some(&info), &settings(2), &[1, 2]);
        assert!(switched.is_empty());
        assert_eq!(notes.len(), 2);
        let (switched, notes) = switches(Some(&info), &settings(1), &[1]);
        assert!(switched.is_empty());
        assert!(
            notes[0].ends_with("the project has one filament, and the reference leaves it out too")
        );
        // Nor is anything written at the layer for it.
        let (codes, notes) = assign(Some(&info), &settings(1), &layers, &[1]).unwrap();
        assert!(codes.is_empty() && notes.is_empty());
    }
}
