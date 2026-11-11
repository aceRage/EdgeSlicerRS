//! A model saved as a project and opened again is the model it was: it
//! places, and slices, to the bytes it did in memory, edits and all; and
//! what its file carried that it does not change goes with it.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use edge_engine::{Bed, Changed, Chosen, Loaded, Model, Overrides, Saving, Transform, Unwatched};
use edge_mesh::Mesh;

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn projects_under(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            projects_under(&path, found);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("3mf"))
        {
            found.push(path);
        }
    }
}

/// The Snapmaker U1's bed, which `mcbrim_test.3mf` is laid out for.
const U1: Bed = Bed {
    min: [0.5, 1.0],
    max: [270.5, 271.0],
};

fn bits(mesh: &Mesh) -> (Vec<[u32; 3]>, Vec<[u32; 3]>) {
    (
        mesh.vertices
            .iter()
            .map(|vertex| vertex.map(f32::to_bits))
            .collect(),
        mesh.triangles.clone(),
    )
}

/// Everything of what is to be sliced that the slice depends on, to the bit.
fn same(a: &Loaded, b: &Loaded, what: &str) {
    assert_eq!(a.bodies.len(), b.bodies.len(), "{what}: bodies");
    for (one, other) in a.bodies.iter().zip(&b.bodies) {
        assert_eq!(one.filament, other.filament, "{what}: filament");
        assert!(bits(&one.solid) == bits(&other.solid), "{what}: solid");
        assert!(
            one.negative.as_ref().map(bits) == other.negative.as_ref().map(bits),
            "{what}: negative"
        );
    }
    assert_eq!(a.painted.len(), b.painted.len(), "{what}: painted");
    for (one, other) in a.painted.iter().zip(&b.painted) {
        assert!(one.facets == other.facets, "{what}: paint");
    }
    assert_eq!(a.objects.len(), b.objects.len(), "{what}: objects");
    for (one, other) in a.objects.iter().zip(&b.objects) {
        assert!(bits(one) == bits(other), "{what}: object");
    }
    // What would reach a printer is not saved.
    let open = |loaded: &Loaded| {
        loaded.settings.clone().map(|mut settings| {
            for key in edge_3mf::SECRETS {
                settings.remove(key);
            }
            settings
        })
    };
    assert_eq!(open(a), open(b), "{what}: settings");
    assert_eq!(a.unread_paint, b.unread_paint, "{what}: paint not read");
    assert_eq!(a.first_layer_sequence, b.first_layer_sequence, "{what}");
    assert_eq!(a.bounds, b.bounds, "{what}: bounds");
}

/// How a model is saved with what it was opened with.
fn as_opened(model: &Model) -> Saving {
    if model.settings().is_some() {
        Saving::Own
    } else {
        Saving::None
    }
}

fn saved(model: &Model, dir: &Path, name: &str) -> Model {
    let path = dir.join(name);
    model.save(&path, &as_opened(model), Some(&U1)).unwrap();
    Model::open(&path).unwrap()
}

/// The plates by number, or the whole model for one without plates.
fn plates(model: &Model) -> Vec<Option<u32>> {
    if model.plates().is_empty() {
        vec![None]
    } else {
        model
            .plates()
            .iter()
            .map(|plate| Some(plate.id()))
            .collect()
    }
}

fn gcode(model: &Model, plate: Option<u32>) -> Vec<u8> {
    let job = edge_engine::prepare(
        model.place(plate).unwrap(),
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    sliced.gcode("2001-01-01 at 00:00:00").unwrap().text
}

fn entries(path: &Path) -> Vec<String> {
    let archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    archive
        .file_names()
        .filter_map(Result::ok)
        .map(|name| name.into_owned())
        .collect()
}

fn entry(path: &Path, name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn every_reference_project_saved_and_opened_again_places_alike() {
    let dir = tempfile::tempdir().unwrap();
    let mut paths = Vec::new();
    for root in ["tests", "resources"] {
        projects_under(&reference(root), &mut paths);
    }
    paths.sort();
    let mut checked = 0;
    for path in &paths {
        let Ok(model) = Model::open(path) else {
            continue;
        };
        let again = saved(&model, dir.path(), "again.3mf");
        let what = path.display().to_string();
        assert_eq!(
            model.plate_list().len().max(1),
            again.plate_list().len(),
            "{what}"
        );
        assert_eq!(model.filaments(), again.filaments(), "{what}");
        for plate in plates(&model) {
            let (Ok(a), Ok(b)) = (model.place(plate), again.place(plate.or(Some(1)))) else {
                continue;
            };
            same(&a, &b, &format!("{what} plate {plate:?}"));
        }
        checked += 1;
    }
    assert!(checked >= 25, "only {checked} projects checked");
}

#[test]
fn a_saved_project_slices_to_the_bytes_its_file_did() {
    let dir = tempfile::tempdir().unwrap();
    for path in [
        "tests/mcbrim_test.3mf",
        "tests/data/support_corpus/twopart_groups.3mf",
    ] {
        let model = Model::open(&reference(path)).unwrap();
        let again = saved(&model, dir.path(), "again.3mf");
        for plate in plates(&model) {
            assert!(
                gcode(&model, plate) == gcode(&again, plate),
                "{path} plate {plate:?}"
            );
        }
    }
}

#[test]
fn an_edited_project_saved_and_opened_again_slices_as_it_did_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = reference("tests/mcbrim_test.3mf");
    let mut model = Model::open(&path).unwrap();
    let bed = Some(&U1);
    let on = |model: &Model, plate: usize| model.plates()[plate].copies().to_vec();
    // A copy on plate 1 moved 10 mm along X.
    let moved = on(&model, 0)[0];
    let (scene, _) = model.in_scene(bed, moved).unwrap();
    let there = scene.then(&Transform::translation([10.0, 0.0, 0.0]));
    model.set_transforms(bed, &[(moved, there)]).unwrap();
    // One on plate 2 deleted.
    let deleted = on(&model, 1)[0];
    model.remove(bed, &[deleted]).unwrap();
    // An object on plate 3 printed in filament 5 and renamed.
    let third = on(&model, 2)[0];
    let object = model
        .objects()
        .iter()
        .find(|object| object.copies().iter().any(|copy| copy.id() == third))
        .unwrap()
        .id();
    model.set_filament(bed, object, 5).unwrap();
    model.rename(bed, object, "Renamed & <kept>").unwrap();

    let file = dir.path().join("edited.3mf");
    model.save(&file, &Saving::Own, bed).unwrap();
    let again = Model::open(&file).unwrap();
    assert_eq!(again.objects().len(), 15);
    assert!(
        again
            .objects()
            .iter()
            .any(|object| object.name() == "Renamed & <kept>" && object.shape().filaments() == [5])
    );
    for plate in plates(&model) {
        same(
            &model.place(plate).unwrap(),
            &again.place(plate).unwrap(),
            &format!("plate {plate:?}"),
        );
        assert!(
            gcode(&model, plate) == gcode(&again, plate),
            "plate {plate:?}"
        );
    }
    // The pictures of plates 1, 2 and 3 no longer show them, and are not
    // saved; the settings and the assembly view are.
    let names = entries(&file);
    assert!(
        !names
            .iter()
            .any(|name| name.starts_with("Metadata/plate_1"))
    );
    assert!(!names.iter().any(|name| name.starts_with("Metadata/top_2")));
    assert!(
        names
            .iter()
            .any(|name| name == "Metadata/project_settings.config")
    );
    let settings = entry(&file, "Metadata/model_settings.config");
    assert!(settings.contains("<assemble_item"));
    assert!(settings.contains(r#"value="Renamed &amp; &lt;kept&gt;""#));
    // Written again it is the same file.
    let twice = dir.path().join("twice.3mf");
    model.save(&twice, &Saving::Own, bed).unwrap();
    assert_eq!(
        std::fs::read(&file).unwrap(),
        std::fs::read(&twice).unwrap()
    );
}

#[test]
fn an_unedited_plate_keeps_its_pictures() {
    let dir = tempfile::tempdir().unwrap();
    let path = reference("tests/mcbrim_test.3mf");
    let mut model = Model::open(&path).unwrap();
    let copy = model.plates()[0].copies()[0];
    let object = model
        .objects()
        .iter()
        .find(|object| object.copies().iter().any(|held| held.id() == copy))
        .unwrap()
        .id();
    // A new name changes nothing that is printed.
    model.rename(Some(&U1), object, "Another").unwrap();
    let file = dir.path().join("renamed.3mf");
    model.save(&file, &Saving::Own, Some(&U1)).unwrap();
    let names = entries(&file);
    for plate in 1..=3 {
        for picture in [
            format!("Metadata/plate_{plate}.png"),
            format!("Metadata/plate_{plate}_small.png"),
            format!("Metadata/top_{plate}.png"),
            format!("Metadata/pick_{plate}.png"),
        ] {
            assert!(names.contains(&picture), "{picture}");
        }
    }
    assert!(entry(&file, "_rels/.rels").contains("/Metadata/plate_1.png"));
}

#[test]
fn what_goes_by_an_objects_place_follows_it_when_one_before_it_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    // Four objects, each with its line of cut information.
    let path = reference("tests/data/bambu_compat_3mf/auto_pa_line_dual.3mf");
    let mut model = Model::open(&path).unwrap();
    let count = model.objects().len();
    assert!(count >= 2);
    let first: Vec<u64> = model.objects()[0].copies().iter().map(|c| c.id()).collect();
    let losses_before = model.save_losses(&Saving::Own);
    model.remove(None, &first).unwrap();
    let file = dir.path().join("fewer.3mf");
    model.save(&file, &Saving::Own, None).unwrap();
    let cut = entry(&file, "Metadata/cut_information.xml");
    // Numbered from 1 again, one fewer.
    assert!(cut.contains(&format!(r#"<object id="{}">"#, count - 1)));
    assert!(!cut.contains(&format!(r#"<object id="{count}">"#)));
    // Its copy's place in the assembly view goes with the object, under
    // its number now.
    assert!(losses_before.is_empty(), "{losses_before:?}");
    let again = Model::open(&file).unwrap();
    assert_eq!(again.objects().len(), count - 1);
}

#[test]
fn a_variable_layer_height_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = reference("tests/data/vlh/vlh_t.3mf");
    let model = Model::open(&path).unwrap();
    let file = dir.path().join("vlh.3mf");
    model.save(&file, &as_opened(&model), None).unwrap();
    let profile = entry(&file, "Metadata/layer_heights_profile.txt");
    assert!(
        profile.starts_with("object_id=1|0.000000;0.200000;3.000000;0.200000;6.000000;0.120000")
    );
}

#[test]
fn settings_settled_here_name_what_was_set_by_hand() {
    let dir = tempfile::tempdir().unwrap();
    let model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let mut settings = model.settings().unwrap().as_ref().clone();
    settings.insert("wall_loops", edge_config::Value::One("5".into()));
    settings.insert("type", edge_config::Value::One("process".into()));
    settings.insert("print_host", edge_config::Value::One("192.168.0.60".into()));
    let changed = Changed {
        process: vec!["wall_loops".into()],
        filaments: vec![Vec::new(), vec!["nozzle_temperature".into()]],
        printer: Vec::new(),
    };
    let file = dir.path().join("settled.3mf");
    model
        .save(
            &file,
            &Saving::Settings {
                settings: edge_engine::project_settings(&settings, &changed),
                presets: false,
            },
            None,
        )
        .unwrap();
    let again = Model::open(&file).unwrap();
    let written = again.settings().unwrap();
    assert_eq!(written.text("wall_loops"), Some("5"));
    // A preset's own keys and a printer's address are not a project's.
    assert!(written.get("type").is_none());
    assert!(written.get("print_host").is_none());
    let groups = written.texts("different_settings_to_system").unwrap();
    // The process, seven filaments, the printer.
    assert_eq!(groups.len(), 9);
    assert_eq!(groups[0], "wall_loops");
    assert_eq!(groups[2], "nozzle_temperature");
    assert_eq!(groups[8], "");
    assert_eq!(written.text("name"), Some("project_settings"));
    assert_eq!(written.text("from"), Some("project"));
}

#[test]
fn a_bare_model_is_saved_standing_in_the_middle_of_the_bed() {
    let dir = tempfile::tempdir().unwrap();
    let mesh = Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
        [-5.0, -5.0, 3.0],
        [5.0, 5.0, 13.0],
    ));
    let stl = dir.path().join("block.stl");
    std::fs::write(&stl, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let model = Model::open(&stl).unwrap();
    let file = dir.path().join("block.3mf");
    model.save(&file, &Saving::None, Some(&U1)).unwrap();
    let again = Model::open(&file).unwrap();
    assert_eq!(again.plates().len(), 1);
    let copy = again.plates()[0].copies()[0];
    let (_, reach) = again.in_scene(Some(&U1), copy).unwrap();
    let [low, high] = reach.unwrap();
    assert_eq!(low[2], 0.0);
    assert!(((low[0] + high[0]) / 2.0 - 135.5).abs() < 1e-9);
    assert!(((low[1] + high[1]) / 2.0 - 136.0).abs() < 1e-9);
    assert!(again.settings().is_none());
    // It slices where slicing the STL puts it.
    let loaded = model.place(None).unwrap();
    let placed = again.place(Some(1)).unwrap();
    assert_eq!(loaded.bodies.len(), placed.bodies.len());
}

#[test]
fn a_files_own_settings_with_what_was_set_over_them_keep_the_rest_as_they_were() {
    let model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let own = model.settings().unwrap();
    let mut settled = own.as_ref().clone();
    settled.insert("wall_loops", edge_config::Value::One("6".into()));
    settled.insert("layer_height", edge_config::Value::One("0.3".into()));
    let changed = Changed {
        process: vec!["wall_loops".into()],
        ..Changed::default()
    };
    let written = edge_engine::own_settings(own, &settled, &changed);
    assert_eq!(written.text("wall_loops"), Some("6"));
    // Not said to be changed, so as the file has it.
    assert_eq!(written.text("layer_height"), own.text("layer_height"));
    let groups = written.texts("different_settings_to_system").unwrap();
    assert!(
        groups[0].split(';').any(|key| key == "wall_loops"),
        "{groups:?}"
    );
}
