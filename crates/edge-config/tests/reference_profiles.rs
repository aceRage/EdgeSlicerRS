//! Checks the loader against every profile the reference slicer ships.

use std::path::PathBuf;

use edge_config::{Kind, Vendor};
use rayon::prelude::*;

fn profiles_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer/resources/profiles");
    assert!(
        dir.is_dir(),
        "reference profiles missing; run `git submodule update --init`"
    );
    dir
}

#[test]
fn every_shipped_preset_resolves() {
    let dir = profiles_dir();
    let vendors = Vendor::list(&dir).unwrap();
    assert!(vendors.len() > 50, "only {} vendors found", vendors.len());

    let (resolved, failures): (usize, Vec<String>) = vendors
        .par_iter()
        .map(|name| {
            let vendor = Vendor::open(&dir, name).unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut resolved = 0;
            let mut failures = Vec::new();
            for kind in Kind::ALL {
                for preset in vendor.names(kind) {
                    match vendor.resolve(kind, preset) {
                        Ok(_) => resolved += 1,
                        Err(error) => failures.push(format!("{name} / {kind} / {preset}: {error}")),
                    }
                }
            }
            (resolved, failures)
        })
        .reduce(
            || (0, Vec::new()),
            |(a, mut x), (b, y)| {
                x.extend(y);
                (a + b, x)
            },
        );

    assert!(
        failures.is_empty(),
        "{} presets failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(resolved > 13_000, "only {resolved} presets resolved");
}

#[test]
fn snapmaker_u1_presets_carry_their_ancestors_settings() {
    let vendor = Vendor::open(&profiles_dir(), "Snapmaker").unwrap();

    // The name project files saved by 2.3.x carry; the preset has since been renamed.
    let process = vendor
        .resolve(Kind::Process, "0.20 Standard @Snapmaker U1 (0.4 nozzle)")
        .unwrap();
    assert_eq!(
        process.text("name"),
        Some("0.20mm Standard @Snapmaker U1 (0.4 nozzle)")
    );
    assert_eq!(process.float("layer_height").unwrap(), Some(0.2));
    assert!(
        process
            .float("initial_layer_print_height")
            .unwrap()
            .is_some()
    );
    assert!(
        process.len() > 100,
        "process resolved to only {} settings",
        process.len()
    );

    let machine = vendor
        .resolve(Kind::Machine, "Snapmaker U1 (0.4 nozzle)")
        .unwrap();
    assert_eq!(machine.text("printer_model"), Some("Snapmaker U1"));
    assert_eq!(
        machine.floats("nozzle_diameter").unwrap().unwrap()[0],
        Some(0.4)
    );
    assert!(machine.float("printable_height").unwrap().is_some());
    assert_eq!(
        machine.texts("printable_area").map(<[String]>::len),
        Some(4)
    );
}

/// The settings the reference takes as plain numbers are those it declares
/// so (`edge_config::plain_number`, from `plain_numbers.txt`): every one
/// declared `coFloat`, `coFloats`, `coInt` or `coInts` in PrintConfig.cpp
/// is on the list, and every one on the list is declared there or made
/// there from one that is (per axis, `machine_max_*`, and per filament,
/// `filament_*`). A bump of the reference that changes them fails here.
#[test]
fn the_plain_numbers_are_those_the_reference_declares() {
    let source = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../reference/EdgeSlicer/src/libslic3r/PrintConfig.cpp"),
    )
    .unwrap();
    let mut declared = std::collections::BTreeMap::new();
    for piece in source.split("this->add").skip(1) {
        let piece = piece.strip_prefix("_nullable").unwrap_or(piece);
        let Some(rest) = piece.strip_prefix("(\"") else {
            continue;
        };
        let Some((key, rest)) = rest.split_once('"') else {
            continue;
        };
        let kind: String = rest
            .trim_start_matches([',', ' '])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        declared.entry(key.to_owned()).or_insert(kind);
    }
    let plain = |kind: &str| matches!(kind, "coFloat" | "coFloats" | "coInt" | "coInts");
    for (key, kind) in &declared {
        assert_eq!(
            edge_config::plain_number(key),
            plain(kind),
            "{key} is declared {kind}"
        );
    }
    let list = include_str!("../src/plain_numbers.txt");
    let lines: Vec<&str> = list.lines().collect();
    assert!(lines.windows(2).all(|pair| pair[0] < pair[1]), "sorted");
    for key in lines {
        let made = ["speed", "acceleration", "jerk"].iter().any(|kind| {
            key.strip_prefix(&format!("machine_max_{kind}_"))
                .is_some_and(|axis| axis.len() == 1)
        });
        let filament = key
            .strip_prefix("filament_")
            .and_then(|own| declared.get(own))
            .is_some_and(|kind| plain(kind));
        assert!(
            declared.get(key).is_some_and(|kind| plain(kind)) || made || filament,
            "{key} is not a plain number in the reference"
        );
    }
}

/// Every number in every shipped preset reads as the reference reads it:
/// a plain number written with a `%` ("50%" of a speed in mm/s) as the
/// number, and one of the settings the reference drops when written so
/// (`handle_legacy`) not at all, so that the value inherited stands.
#[test]
fn every_shipped_presets_numbers_read() {
    let dir = profiles_dir();
    let vendors = Vendor::list(&dir).unwrap();
    let (presets, with_percent, failures) = vendors
        .par_iter()
        .map(|name| {
            let vendor = Vendor::open(&dir, name).unwrap();
            let (mut presets, mut with_percent, mut failures) = (0usize, 0usize, Vec::new());
            // A printer model's file describes the model, not settings.
            for kind in [Kind::Machine, Kind::Process, Kind::Filament] {
                for preset in vendor.names(kind) {
                    let Ok(settings) = vendor.resolve(kind, preset) else {
                        continue;
                    };
                    presets += 1;
                    let mut percent = false;
                    for (key, value) in settings.iter() {
                        if !edge_config::plain_number(key) {
                            continue;
                        }
                        let items = match value {
                            edge_config::Value::One(text) => std::slice::from_ref(text),
                            edge_config::Value::Many(items) => items.as_slice(),
                        };
                        // Only the percentages: some profiles have other
                        // slips ("0,0" in one entry), which are not this.
                        if !items.iter().any(|item| item.contains('%')) {
                            continue;
                        }
                        percent = true;
                        if let Err(error) = settings.floats(key) {
                            failures.push(format!("{name} / {kind} / {preset}: {error}"));
                        }
                    }
                    with_percent += usize::from(percent);
                    for key in [
                        "initial_layer_print_height",
                        "initial_layer_speed",
                        "internal_solid_infill_speed",
                        "top_surface_speed",
                        "support_interface_speed",
                        "outer_wall_speed",
                        "support_object_xy_distance",
                    ] {
                        if settings
                            .texts(key)
                            .is_some_and(|items| items.iter().any(|item| item.contains('%')))
                        {
                            failures.push(format!(
                                "{name} / {kind} / {preset}: {key} kept a percentage"
                            ));
                        }
                    }
                }
            }
            (presets, with_percent, failures)
        })
        .reduce(
            || (0, 0, Vec::new()),
            |(a, b, mut x), (c, d, y)| {
                x.extend(y);
                (a + c, b + d, x)
            },
        );
    assert!(
        failures.is_empty(),
        "{} failed:\n{}",
        failures.len(),
        failures[..failures.len().min(20)].join("\n")
    );
    eprintln!("{presets} presets, {with_percent} with a plain number written as a percentage");
    assert!(presets > 10_000);
    assert!(with_percent > 0);
}
