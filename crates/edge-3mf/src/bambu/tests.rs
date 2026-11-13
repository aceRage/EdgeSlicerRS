//! The Bambu form of a project file: what changes and what does not.

use std::{io::Read, path::PathBuf};

use super::*;
use crate::{Project, Save};

fn reference() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference/EdgeSlicer")
}

fn entries(path: &Path) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    (0..archive.len())
        .map(|at| {
            let mut entry = archive.by_index(at).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            (entry.name().unwrap().into_owned(), bytes)
        })
        .collect()
}

fn text<'a>(all: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
    let (_, bytes) = all
        .iter()
        .find(|(entry, _)| entry == name)
        .unwrap_or_else(|| panic!("no {name}"));
    std::str::from_utf8(bytes).unwrap()
}

fn own_save(project: &Project) -> Save<'_> {
    let settings = project
        .kept
        .as_ref()
        .and_then(|kept| kept.settings_text())
        .map(|text| crate::own_settings_text(text).unwrap());
    let filaments = project
        .settings
        .as_ref()
        .and_then(|settings| settings.texts("filament_colour").map(<[String]>::len))
        .unwrap_or(1);
    Save {
        project,
        settings,
        presets: true,
        inside: None,
        filaments,
    }
}

#[test]
fn the_bambu_form_carries_the_converted_settings_under_bambus_tag() {
    let project = Project::open(reference().join("tests/mcbrim_test.3mf")).unwrap();
    // Something of each kind to convert: a renamed key, an enum, one of
    // ours alone, an object's and a part's own, a height range's, a
    // plate's bed type, this app's own metadata.
    let mut project = project.clone();
    {
        let object = &mut project.objects[0];
        let mut own = object.kept.as_deref().cloned().unwrap();
        own.entries.push(Entry::Meta {
            key: "sparse_infill_pattern".into(),
            value: "rectilinear".into(),
        });
        own.entries.push(Entry::Meta {
            key: "layerling_design".into(),
            value: "abc".into(),
        });
        own.entries.push(Entry::Meta {
            key: "make_overhang_printable".into(),
            value: "1".into(),
        });
        own = own.with_height_ranges(&[kept::LayerRange {
            min_z: 1.0,
            max_z: 2.0,
            settings: vec![
                ("only_one_wall_top".into(), "1".into()),
                ("wall_loops".into(), "4".into()),
            ],
        }]);
        object.kept = Some(Arc::new(own));
        let part = &mut object.parts[0];
        let mut own = part.kept.as_deref().cloned().unwrap();
        own.entries.insert(
            0,
            Entry::Meta {
                key: "infill_anchor".into(),
                value: "2".into(),
            },
        );
        own.entries.push(Entry::Meta {
            key: "cad_body_file".into(),
            value: "Metadata/cad_bodies/body_1.bin".into(),
        });
        part.kept = Some(Arc::new(own));
        let plate = &mut project.plates[0];
        let mut own = plate
            .kept
            .as_deref()
            .cloned()
            .unwrap_or_else(|| kept::PlateKept {
                id: plate.id,
                entries: Vec::new(),
                pictures: Default::default(),
            });
        own.entries.retain(|entry| entry.key() != Some("bed_type"));
        own.entries.push(Entry::Meta {
            key: "bed_type".into(),
            value: "Textured Cool Plate".into(),
        });
        own.entries.push(Entry::Meta {
            key: "edgeslicer_print_history".into(),
            value: "x".into(),
        });
        plate.kept = Some(Arc::new(own));
    }
    let save = own_save(&project);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bambu.3mf");
    let (written, report) = write_bambu(&save, &path, Layout::Project).unwrap();
    let all = entries(&path);
    assert_eq!(
        written.entries,
        all.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>()
    );
    // Bambu's tag, and its line in the settings and the slice information.
    let model = text(&all, "3D/3dmodel.model");
    assert!(
        model.contains(r#"<metadata name="Application">BambuStudio-02.08.00.00</metadata>"#),
        "{model}"
    );
    assert!(
        text(&all, "Metadata/slice_info.config")
            .contains(r#"key="X-BBL-Client-Version" value="02.08.00.00""#)
    );
    let settings: serde_json::Value =
        serde_json::from_str(text(&all, "Metadata/project_settings.config")).unwrap();
    assert_eq!(settings["version"], "02.08.00.00");
    assert_eq!(settings["name"], "project_settings");
    assert_eq!(settings["from"], "project");
    assert!(settings.get("print_compatible_printers").is_some());
    assert_eq!(settings["filament_self_index"].as_array().unwrap().len(), 7);
    // The object's, the part's and the range's own, converted.
    let model_settings = text(&all, "Metadata/model_settings.config");
    assert!(
        model_settings.contains(r#"<metadata key="sparse_infill_pattern" value="zig-zag"/>"#),
        "{model_settings}"
    );
    assert!(model_settings.contains(r#"<metadata key="sparse_infill_anchor" value="2"/>"#));
    assert!(!model_settings.contains("make_overhang_printable"));
    assert!(!model_settings.contains("layerling_design"));
    assert!(!model_settings.contains("cad_body_file"));
    assert!(!model_settings.contains("edgeslicer_print_history"));
    // A plate type Bambu has not: left out.
    assert!(!model_settings.contains("Textured Cool Plate"));
    let ranges = text(&all, "Metadata/layer_config_ranges.xml");
    assert!(
        ranges.contains(r#"<option opt_key="top_one_wall_type">all top</option>"#),
        "{ranges}"
    );
    assert!(ranges.contains(r#"<option opt_key="wall_loops">4</option>"#));
    assert!(report.dropped.contains("make_overhang_printable"));
    assert_eq!(report.renamed["infill_anchor"], "sparse_infill_anchor");
    assert_eq!(report.renamed["only_one_wall_top"], "top_one_wall_type");

    // The project's own form is what it always was.
    let own = dir.path().join("own.3mf");
    crate::write(&save, &own).unwrap();
    let all = entries(&own);
    assert!(
        text(&all, "3D/3dmodel.model")
            .contains(r#"<metadata name="Application">EdgeSlicer-2.4.9.0</metadata>"#)
    );
    let model_settings = text(&all, "Metadata/model_settings.config");
    assert!(
        model_settings.contains("layerling_design")
            && model_settings.contains("make_overhang_printable")
    );
    assert!(text(&all, "Metadata/slice_info.config").contains(r#"value="02.04.09.00""#));
}

#[test]
fn presets_inside_are_converted_and_named_for_bambu() {
    let project =
        Project::open(reference().join("tests/data/support_corpus/twopart_groups.3mf")).unwrap();
    let presets = vec![
        InsidePreset {
            kind: edge_config::Kind::Process,
            name: "Project process".into(),
            inherits: Some("0.20mm Standard @BBL A1M".into()),
            version: Some("2.4.9.0".into()),
            own: Settings::from_json(r#"{"wall_loops": "5", "sparse_infill_pattern": "rectilinear", "make_overhang_printable": "1"}"#).unwrap(),
        },
        InsidePreset {
            kind: edge_config::Kind::Filament,
            name: "Project PLA".into(),
            inherits: None,
            version: None,
            own: Settings::from_json(r#"{"nozzle_temperature": ["231"]}"#).unwrap(),
        },
    ];
    let mut save = own_save(&project);
    save.inside = Some(&presets);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bambu.3mf");
    let (_, report) = write_bambu(&save, &path, Layout::Project).unwrap();
    let all = entries(&path);
    let process: serde_json::Value =
        serde_json::from_str(text(&all, "Metadata/process_settings_1.config")).unwrap();
    assert_eq!(process["name"], "Project process");
    assert_eq!(process["from"], "project");
    assert_eq!(process["version"], "02.08.00.00");
    assert_eq!(process["inherits"], "0.20mm Standard @BBL A1M");
    assert_eq!(process["print_settings_id"], "Project process");
    assert_eq!(process["sparse_infill_pattern"], "zig-zag");
    assert!(process.get("make_overhang_printable").is_none());
    let filament: serde_json::Value =
        serde_json::from_str(text(&all, "Metadata/filament_settings_1.config")).unwrap();
    assert_eq!(
        filament["filament_settings_id"],
        serde_json::json!(["Project PLA"])
    );
    assert_eq!(filament["nozzle_temperature"], serde_json::json!(["231"]));
    assert_eq!(filament["filament_self_index"], serde_json::json!(["1"]));
    assert!(
        report
            .notes
            .iter()
            .any(|note| note.starts_with("preset Project process: "))
    );
}

#[test]
fn this_apps_own_entries_are_left_out() {
    assert!(left_out("Metadata/cad_bodies/body_1.bin"));
    assert!(left_out("Metadata/edgeslicer_height_patterns.json"));
    assert!(left_out("Metadata/showcase/scenes.json"));
    assert!(!left_out("Metadata/cut_information.xml"));
}
