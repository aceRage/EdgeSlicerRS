//! Settings of an object, a part and a height range of an object, set on
//! the model, saved and opened again; and what is refused.

use std::{io::Read, path::Path};

use edge_engine::{Bed, LayerRange, Model, Saving};

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

const U1: Bed = Bed {
    min: [0.5, 1.0],
    max: [270.5, 271.0],
};

fn saving(model: &Model) -> Saving {
    if model.settings().is_some() {
        Saving::Own
    } else {
        Saving::None
    }
}

fn entry(path: &Path, name: &str) -> Option<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .ok()?
        .read_to_string(&mut text)
        .unwrap();
    Some(text)
}

fn range(min_z: f64, max_z: f64, settings: &[(&str, &str)]) -> LayerRange {
    LayerRange {
        min_z,
        max_z,
        settings: settings
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
    }
}

fn twopart() -> Model {
    Model::open(&reference("tests/data/support_corpus/twopart_groups.3mf")).unwrap()
}

#[test]
fn an_objects_a_parts_and_a_ranges_settings_are_saved_and_read_back() {
    let mut model = twopart();
    let bed = Some(&U1);
    let object = model.objects()[0].id();
    let parts = model.objects()[0].shape().part_ids().to_vec();
    let stamp = model.objects()[0].shape().stamp();
    model
        .set_object_settings(bed, object, &[("wall_loops".into(), Some("5".into()))])
        .unwrap();
    assert_ne!(
        model.objects()[0].shape().stamp(),
        stamp,
        "the slice is let go"
    );
    model
        .set_part_settings(
            bed,
            object,
            parts[1],
            &[("sparse_infill_density".into(), Some("60%".into()))],
        )
        .unwrap();
    let ranges = vec![
        range(0.0, 2.0, &[("extruder", "0"), ("layer_height", "0.1")]),
        range(4.0, 6.5, &[("layer_height", "0.28"), ("wall_loops", "1")]),
    ];
    model
        .set_height_ranges(bed, object, ranges.iter().rev().cloned().collect())
        .unwrap();
    assert_eq!(model.objects()[0].layer_ranges(), ranges, "put in order");

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("set.3mf");
    model.save(&file, &saving(&model), bed).unwrap();
    let again = Model::open(&file).unwrap();
    let object_again = &again.objects()[0];
    assert!(
        object_again
            .settings()
            .contains(&("wall_loops".to_owned(), "5".to_owned()))
    );
    assert!(
        object_again.shape().parts()[1]
            .settings()
            .contains(&("sparse_infill_density".to_owned(), "60%".to_owned()))
    );
    assert_eq!(object_again.layer_ranges(), ranges);
    let xml = entry(&file, "Metadata/layer_config_ranges.xml").unwrap();
    assert_eq!(
        xml,
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<objects>\n <object id=\"1\">\n  <range min_z=\"0\" max_z=\"2\">\n   <option opt_key=\"extruder\">0</option>\n   <option opt_key=\"layer_height\">0.1</option>\n  </range>\n  <range min_z=\"4\" max_z=\"6.5\">\n   <option opt_key=\"layer_height\">0.28</option>\n   <option opt_key=\"wall_loops\">1</option>\n  </range>\n </object>\n</objects>\n"
    );
    let settings = entry(&file, "Metadata/model_settings.config").unwrap();
    assert!(settings.contains(r#"<metadata key="wall_loops" value="5"/>"#));
    assert!(settings.contains(r#"<metadata key="sparse_infill_density" value="60%"/>"#));

    // Saved again, the same bytes; given back, the file it was.
    let twice = dir.path().join("twice.3mf");
    again.save(&twice, &saving(&again), bed).unwrap();
    model.save(&file, &saving(&model), bed).unwrap();
    assert!(std::fs::read(&file).unwrap() == std::fs::read(&twice).unwrap());

    let mut back = model.clone();
    back.set_object_settings(bed, object, &[("wall_loops".into(), None)])
        .unwrap();
    back.set_part_settings(
        bed,
        object,
        parts[1],
        &[("sparse_infill_density".into(), None)],
    )
    .unwrap();
    back.set_height_ranges(bed, object, Vec::new()).unwrap();
    let plain = dir.path().join("plain.3mf");
    let untouched = dir.path().join("untouched.3mf");
    back.save(&plain, &saving(&back), bed).unwrap();
    twopart().save(&untouched, &saving(&back), bed).unwrap();
    assert!(std::fs::read(&plain).unwrap() == std::fs::read(&untouched).unwrap());
}

#[test]
fn a_bare_models_object_given_a_setting_keeps_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let cube = dir.path().join("cube.stl");
    let mut text = String::from("solid cube\n");
    let corners = [
        [0.0, 0.0, 0.0],
        [10.0, 0.0, 0.0],
        [10.0, 10.0, 0.0],
        [0.0, 10.0, 0.0],
        [0.0, 0.0, 10.0],
        [10.0, 0.0, 10.0],
        [10.0, 10.0, 10.0],
        [0.0, 10.0, 10.0],
    ];
    let faces = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    for face in faces {
        text.push_str(" facet normal 0 0 0\n  outer loop\n");
        for corner in face {
            let [x, y, z] = corners[corner];
            text.push_str(&format!("   vertex {x} {y} {z}\n"));
        }
        text.push_str("  endloop\n endfacet\n");
    }
    text.push_str("endsolid cube\n");
    std::fs::write(&cube, text).unwrap();
    let mut model = Model::open(&cube).unwrap();
    let object = model.objects()[0].id();
    model
        .set_object_settings(
            Some(&U1),
            object,
            &[("wall_loops".into(), Some("5".into()))],
        )
        .unwrap();
    assert_eq!(model.objects()[0].name(), "cube.stl");
    let file = dir.path().join("cube.3mf");
    model.save(&file, &Saving::None, Some(&U1)).unwrap();
    let again = Model::open(&file).unwrap();
    assert_eq!(again.objects()[0].name(), "cube.stl");
    assert_eq!(
        again.objects()[0].settings(),
        vec![("wall_loops".to_owned(), "5".to_owned())]
    );
}

#[test]
fn ranges_that_overlap_or_end_where_they_start_and_helpers_settings_are_refused() {
    let mut model = twopart();
    let object = model.objects()[0].id();
    let was = model.clone();
    for ranges in [
        vec![range(0.0, 2.0, &[]), range(1.0, 3.0, &[])],
        vec![range(2.0, 2.0, &[])],
        vec![range(-1.0, 2.0, &[])],
        vec![range(f64::NAN, 2.0, &[])],
    ] {
        assert!(model.set_height_ranges(Some(&U1), object, ranges).is_err());
    }
    // Touching is not overlapping.
    model
        .set_height_ranges(
            Some(&U1),
            object,
            vec![range(0.0, 2.0, &[]), range(2.0, 3.0, &[])],
        )
        .unwrap();
    let kinds: Vec<_> = was.objects()[0]
        .shape()
        .parts()
        .iter()
        .map(|part| part.kind)
        .collect();
    let ids = was.objects()[0].shape().part_ids().to_vec();
    for (kind, id) in kinds.iter().zip(&ids) {
        let mut tried = was.clone();
        let done = tried.set_part_settings(
            Some(&U1),
            object,
            *id,
            &[("wall_loops".into(), Some("3".into()))],
        );
        let takes = matches!(
            kind,
            edge_engine::PartKind::Normal | edge_engine::PartKind::Modifier
        );
        assert_eq!(done.is_ok(), takes, "{kind:?}");
    }
}
