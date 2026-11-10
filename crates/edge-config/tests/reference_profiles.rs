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
