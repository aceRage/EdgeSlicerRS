//! Opening a model to slice it as the reference's command line does: a
//! model without plates of the reference's kind is arranged on the bed
//! first, a project of its kind is left where it stands.

use std::path::{Path, PathBuf};

use edge_config::Settings;
use edge_engine::{Arrange, Bed, Chosen, Model, Overrides, Saving, open_to_slice};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// A box of `size` mm with a corner at `at`, as an STL file in `dir`.
fn block(dir: &Path, name: &str, at: [f32; 3], size: [f32; 3]) -> PathBuf {
    let high = [at[0] + size[0], at[1] + size[1], at[2] + size[2]];
    let mesh = Mesh::from_triangle_soup(cuboid(at, high));
    let path = dir.join(name);
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    path
}

/// A printer of a 200 by 200 mm bed, `structure` its frame ("i3" turns
/// what is arranged to run along Y).
fn printer(structure: &str) -> Chosen {
    let settings = Settings::from_json(&format!(
        r#"{{"machine_start_gcode": "G28", "printable_area": ["0x0", "200x0", "200x200", "0x200"],
            "nozzle_diameter": ["0.4"], "printable_height": "200", "printer_structure": "{structure}",
            "skirt_loops": "0", "brim_type": "no_brim"}}"#
    ))
    .unwrap();
    Chosen {
        machine: Some(("printer".into(), settings)),
        ..Chosen::default()
    }
}

fn extent(bounds: ([f32; 3], [f32; 3])) -> [f64; 4] {
    let (low, high) = bounds;
    [low[0], low[1], high[0], high[1]].map(f64::from)
}

fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(&b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn a_bare_model_is_arranged_in_the_middle_of_the_bed() {
    let dir = tempfile::tempdir().unwrap();
    let stl = block(dir.path(), "box.stl", [10.0, 20.0, 0.0], [25.0, 20.0, 15.0]);
    let chosen = printer("corexy");
    let opened = open_to_slice(&stl, None, Arrange::Auto, &chosen, &Overrides::default()).unwrap();
    assert!(opened.arranged.is_some());
    assert_eq!(opened.plate, Some((1, 1)));
    assert!(opened.loaded.placed);
    let at = extent(opened.loaded.bounds);
    assert!(near(at, [87.5, 90.0, 112.5, 110.0]), "{at:?}");

    // An i3 has it turned to run along Y, as the reference's command
    // line turns it (`align_to_y_axis` from `printer_structure`).
    let chosen = printer("i3");
    let opened = open_to_slice(&stl, None, Arrange::Auto, &chosen, &Overrides::default()).unwrap();
    let at = extent(opened.loaded.bounds);
    assert!(near(at, [90.0, 87.5, 110.0, 112.5]), "{at:?}");

    // Arranged, it has plates, and only those.
    let refused = open_to_slice(&stl, Some(2), Arrange::Auto, &chosen, &Overrides::default())
        .err()
        .unwrap()
        .to_string();
    assert!(refused.contains("no plate 2"), "{refused}");
}

#[test]
fn arranging_off_leaves_a_model_as_the_file_has_it() {
    let dir = tempfile::tempdir().unwrap();
    let stl = block(dir.path(), "box.stl", [10.0, 20.0, 0.0], [25.0, 20.0, 15.0]);
    let chosen = printer("corexy");
    let opened = open_to_slice(&stl, None, Arrange::Off, &chosen, &Overrides::default()).unwrap();
    assert!(opened.arranged.is_none());
    assert!(!opened.loaded.placed);
    assert!(near(extent(opened.loaded.bounds), [10.0, 20.0, 35.0, 40.0]));
    // With no printer there is no bed to arrange on.
    let opened = open_to_slice(
        &stl,
        None,
        Arrange::Auto,
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    assert!(opened.arranged.is_none());
}

#[test]
fn a_plain_project_is_arranged_and_its_objects_numbered_in_the_order_placed() {
    let dir = tempfile::tempdir().unwrap();
    let bed = Bed {
        min: [0.0, 0.0],
        max: [200.0, 200.0],
    };
    let small = Model::open(&block(dir.path(), "a.stl", [0.0; 3], [20.0, 20.0, 20.0])).unwrap();
    let large = Model::open(&block(dir.path(), "b.stl", [0.0; 3], [40.0, 30.0, 10.0])).unwrap();
    let mut model = Model::new();
    model.import(Some(&bed), &small, None).unwrap();
    model.import(Some(&bed), &large, None).unwrap();
    // Written without printer settings: not a project of the
    // reference's kind, whoever wrote it.
    let file = dir.path().join("two.3mf");
    model.save(&file, &Saving::None, Some(&bed)).unwrap();

    let chosen = printer("corexy");
    let opened = open_to_slice(&file, None, Arrange::Auto, &chosen, &Overrides::default()).unwrap();
    assert!(opened.arranged.is_some());
    let labels: Vec<(usize, &str)> = (opened.loaded.labels.iter())
        .map(|label| (label.object, label.name.as_str()))
        .collect();
    // The larger is placed first, and so numbered first.
    assert_eq!(labels, [(0, "b.stl"), (1, "a.stl")]);
    let objects = &opened.loaded.objects;
    let boxes: Vec<[f64; 4]> = objects
        .iter()
        .map(|mesh| extent(mesh.bounds().unwrap()))
        .collect();
    let [a, b] = [boxes[0], boxes[1]];
    let apart = a[2] <= b[0] || b[2] <= a[0] || a[3] <= b[1] || b[3] <= a[1];
    assert!(apart, "{boxes:?}");
    // The pile is in the middle of the bed.
    let pile = [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ];
    assert!(((pile[0] + pile[2]) / 2.0 - 100.0).abs() < 1e-3, "{pile:?}");
    assert!(((pile[1] + pile[3]) / 2.0 - 100.0).abs() < 1e-3, "{pile:?}");

    // The same every time.
    let again = open_to_slice(&file, None, Arrange::Auto, &chosen, &Overrides::default()).unwrap();
    assert_eq!(again.loaded.bounds, opened.loaded.bounds);
}

#[test]
fn a_project_of_the_references_kind_stands_where_it_is() {
    let file = reference("tests/data/support_corpus/onepart_group.3mf");
    let as_read = edge_engine::open(&file, None).unwrap().place().unwrap();
    let opened = open_to_slice(
        &file,
        None,
        Arrange::Auto,
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    assert!(opened.arranged.is_none());
    assert_eq!(opened.loaded.bounds, as_read.bounds);
    // Arranging its plate first is not done: said, not skipped.
    let refused = open_to_slice(
        &file,
        None,
        Arrange::On,
        &Chosen::default(),
        &Overrides::default(),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(refused.contains("not done yet"), "{refused}");
}

#[test]
fn the_option_reads_as_the_references_does() {
    assert_eq!(Arrange::from_option(0), Arrange::Off);
    assert_eq!(Arrange::from_option(1), Arrange::On);
    assert_eq!(Arrange::from_option(2), Arrange::Auto);
    assert_eq!(Arrange::from_option(-1), Arrange::Auto);
}
