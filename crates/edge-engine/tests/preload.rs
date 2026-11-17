//! Every filament loaded at the start (`preload_all_filaments`): what the
//! reference does with it on a printer that does not group filaments onto
//! nozzles of two kinds. It refuses a plate with no prime tower, in its own
//! words, and otherwise prints as without it.

use std::path::{Path, PathBuf};

use edge_config::{Kind, Settings, Vendor};
use edge_engine::{Chosen, Model, Overrides, Unwatched};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// Two blocks side by side, 4 mm tall; with `two`, the second in filament 2.
fn blocks(dir: &Path, two: bool) -> Model {
    let soup = cuboid([110.0, 120.0, 0.0], [125.0, 135.0, 4.0])
        .into_iter()
        .chain(cuboid([130.0, 120.0, 0.0], [145.0, 135.0, 4.0]));
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join("blocks.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    if two {
        let object = model.objects()[0].id();
        model.split_to_parts(None, object).unwrap();
        let second = model.objects()[0].shape().part_ids()[1];
        model
            .set_part_filament(None, object, &[second], Some(2))
            .unwrap();
    }
    model
}

fn u1() -> Chosen {
    let vendor = Vendor::open(&reference("resources/profiles"), "Snapmaker").unwrap();
    let preset = |kind, name: &str| (name.to_owned(), vendor.resolve(kind, name).unwrap());
    let filament = preset(Kind::Filament, "Generic PLA @U1 0.4 nozzle");
    Chosen {
        machine: Some(preset(Kind::Machine, "Snapmaker U1 (0.4 nozzle)")),
        process: Some(preset(
            Kind::Process,
            "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
        )),
        filaments: vec![filament.clone(), filament],
    }
}

/// The G-code without its first line, which carries the time, or the
/// error the slice gives.
fn slice(model: &Model, set: &str) -> Result<String, String> {
    let overrides = Overrides {
        settings: Settings::from_json(set).unwrap(),
        ..Overrides::default()
    };
    let job = edge_engine::prepare(model.place(None).unwrap(), &u1(), &overrides).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).map_err(|error| error.to_string())?;
    let gcode = sliced
        .gcode("2001-01-01 at 00:00:00")
        .map_err(|error| error.to_string())?;
    let text = String::from_utf8(gcode.text).unwrap();
    // The settings block names the setting itself.
    Ok(text
        .lines()
        .filter(|line| !line.starts_with("; preload_all_filaments"))
        .collect::<Vec<_>>()
        .join("\n"))
}

const REFUSED: &str = "\"Preload all filaments\" requires the prime tower";

#[test]
fn a_plate_without_a_prime_tower_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let two = blocks(dir.path(), true);
    let error = slice(
        &two,
        r#"{"preload_all_filaments": "1", "enable_prime_tower": "0"}"#,
    )
    .unwrap_err();
    assert!(error.contains(REFUSED), "{error}");
    // With the tower, the U1 prints as it would without the setting.
    let on = slice(
        &two,
        r#"{"preload_all_filaments": "1", "enable_prime_tower": "1"}"#,
    )
    .unwrap();
    let off = slice(
        &two,
        r#"{"preload_all_filaments": "0", "enable_prime_tower": "1"}"#,
    )
    .unwrap();
    assert!(on == off, "the setting changed the U1's G-code");
}

#[test]
fn one_filament_is_not_refused_for_the_tower_it_does_not_need() {
    // The reference turns the tower off for a plate in one filament and
    // then refuses it for want of the tower; here it is sliced. With the
    // tower off in the profile it is refused, as there.
    let dir = tempfile::tempdir().unwrap();
    let one = blocks(dir.path(), false);
    slice(
        &one,
        r#"{"preload_all_filaments": "1", "enable_prime_tower": "1"}"#,
    )
    .unwrap();
    let error = slice(
        &one,
        r#"{"preload_all_filaments": "1", "enable_prime_tower": "0"}"#,
    )
    .unwrap_err();
    assert!(error.contains(REFUSED), "{error}");
}
