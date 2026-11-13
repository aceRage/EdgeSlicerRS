//! The sliced file and the generic 3MF, written through the session.

use std::{io::Read, path::Path, sync::Arc};

use serde_json::{Value, json};

use super::*;

const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";

fn reference(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

fn sliced(session: &Arc<Session>, path: &str, plate: Option<u32>) -> u64 {
    let id = ask(session, "project_open", json!({ "path": reference(path) }))["id"]
        .as_u64()
        .unwrap();
    if let Some(plate) = plate {
        ask(session, "project_set", json!({ "plate": plate }));
    }
    ask(session, "slice", json!({}));
    loop {
        let status = ask(session, "status", json!({ "project": id }));
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            return id;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
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

#[test]
fn a_plate_sliced_is_exported_as_the_reference_lays_it_out() {
    let session = Arc::new(Session::new(None));
    let id = sliced(&session, SMALL, None);
    let dir = tempfile::tempdir().unwrap();
    let suggested = ask(&session, "export_sliced", json!({ "suggest": true }));
    assert!(
        suggested["suggested"]
            .as_str()
            .unwrap()
            .ends_with("twopart_groups.gcode.3mf"),
        "{suggested}"
    );
    let plates = ask(&session, "export_sliced_plates", json!({ "project": id }));
    assert_eq!(plates["plates"], json!([1]));
    assert_eq!(plates["all_ready"], true);
    let number = plates["copies"][0]["number"].as_u64().unwrap();
    // One pixel of a PNG is enough to be written as it is given.
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let picture = json!({ "plate": png, "small": png, "no_light": png, "top": png, "pick": png });
    let out = dir.path().join("plate");
    let written = ask(
        &session,
        "export_sliced",
        json!({ "path": out, "pictures": { "1": picture } }),
    );
    let path = dir.path().join("plate.3mf");
    assert_eq!(written["path"], shown(&path));
    let names: Vec<String> = entries(&path).into_iter().map(|(name, _)| name).collect();
    // The reference's own export of this project, its entries in its order.
    assert_eq!(
        names,
        [
            "[Content_Types].xml",
            "Metadata/plate_1.png",
            "Metadata/plate_1_small.png",
            "Metadata/plate_no_light_1.png",
            "Metadata/top_1.png",
            "Metadata/pick_1.png",
            "Metadata/plate_1.json",
            "3D/3dmodel.model",
            "Metadata/project_settings.config",
            "Metadata/plate_1.gcode.md5",
            "Metadata/plate_1.gcode",
            "Metadata/_rels/model_settings.config.rels",
            "Metadata/model_settings.config",
            "Metadata/slice_info.config",
            "Metadata/filament_sequence.json",
            "_rels/.rels",
        ]
    );
    let all = entries(&path);
    let gcode = &all
        .iter()
        .find(|(name, _)| name == "Metadata/plate_1.gcode")
        .unwrap()
        .1;
    assert_eq!(
        text(&all, "Metadata/plate_1.gcode.md5"),
        edge_engine::gcode_md5(gcode)
    );
    let info = text(&all, "Metadata/slice_info.config");
    assert!(
        info.contains(&format!(
            r#"<object identify_id="{number}" name="assembly" skipped="false" />"#
        )),
        "{info}"
    );
    assert!(
        info.contains(r##"<filament id="1" tray_info_idx="GFSL99" type="PLA" color="#F2754E""##)
    );
    let settings = text(&all, "Metadata/model_settings.config");
    assert!(settings.contains(r#"<metadata key="gcode_file" value="Metadata/plate_1.gcode"/>"#));
    assert!(settings.contains(r#"<metadata key="pick_file" value="Metadata/pick_1.png"/>"#));
    assert!(!settings.contains("<object"));
    assert!(text(&all, "3D/3dmodel.model").contains("BambuStudio-"));
    assert!(text(&all, "_rels/.rels").contains("/Metadata/plate_1_small.png"));
    // There now: asked about first.
    let again = ask(&session, "export_sliced", json!({ "path": path }));
    assert_eq!(again["exists"], true);
}

#[test]
fn every_plate_sliced_goes_in_one_file() {
    let session = Arc::new(Session::new(None));
    let id = sliced(&session, THREE_PLATES, Some(2));
    // One plate of three sliced: "Export all" is not ready yet, and the
    // plate shown can be exported.
    let plates = ask(
        &session,
        "export_sliced_plates",
        json!({ "project": id, "all": true }),
    );
    assert_eq!(plates["all_ready"], false);
    assert_eq!(plates["all_plates"], json!([1, 2, 3]));
    let menu = ask(&session, "menu", json!({ "tab": "preview" }));
    let entry = |id: &str| {
        fn walk(items: &Value, id: &str) -> Option<Value> {
            for item in items.as_array()? {
                if item["id"] == id {
                    return Some(item.clone());
                }
                if let Some(found) = walk(&item["items"], id) {
                    return Some(found);
                }
            }
            None
        }
        walk(&menu["menus"], id).unwrap()
    };
    assert_eq!(entry("export.plate_sliced")["enabled"], true);
    assert_eq!(entry("export.all_sliced")["enabled"], false);
    assert_eq!(entry("export.generic_3mf")["enabled"], true);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two.gcode.3mf");
    ask(&session, "export_sliced", json!({ "path": path }));
    let all = entries(&path);
    let names: Vec<&str> = all.iter().map(|(name, _)| name.as_str()).collect();
    assert!(
        names.contains(&"Metadata/plate_2.gcode") && !names.contains(&"Metadata/plate_1.gcode")
    );
    let sequence = text(&all, "Metadata/filament_sequence.json");
    assert!(sequence.starts_with(
        r#"{"plate_1":{"nozzle_sequence":[],"optimal_assignment":[],"sequence":[]},"plate_2":{"#
    ));
    // A U1: each filament from its own toolhead, round again past the fourth.
    assert!(
        text(&all, "Metadata/slice_info.config")
            .contains(r#"<metadata key="filament_maps" value="1 2 3 4 1 2 3"/>"#)
    );
}

#[test]
fn the_generic_3mf_has_every_mesh_in_the_model_file() {
    let session = Arc::new(Session::new(None));
    ask(
        &session,
        "project_open",
        json!({ "path": reference(SMALL) }),
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("generic.3mf");
    ask(&session, "export_generic", json!({ "path": path }));
    let all = entries(&path);
    let model = text(&all, "3D/3dmodel.model");
    assert!(
        model.contains("<mesh>")
            && !model.contains("p:UUID")
            && !model.contains("requiredextensions")
    );
    assert!(!all.iter().any(|(name, _)| name.starts_with("3D/Objects/")));
    assert!(
        all.iter()
            .any(|(name, _)| name == "Metadata/project_settings.config")
    );
    // The project is still its own file, and unsaved as it was.
    let project = ask(&session, "project", Value::Null);
    assert_eq!(project["file"], "twopart_groups.3mf");
}
