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

// ---- the parts of an object, edited and saved ----

/// A part as a saved project holds it: everything of it a slice or the
/// reference reads, the mesh to the bit.
#[derive(Debug, PartialEq)]
struct HeldPart {
    name: Option<String>,
    kind: edge_engine::PartKind,
    filament: usize,
    own_filament: Option<usize>,
    transform: [[u64; 3]; 4],
    mesh: (Vec<[u32; 3]>, Vec<[u32; 3]>),
    paint: Option<Vec<edge_3mf::Facet>>,
    seam: Option<Vec<edge_3mf::SeamFacet>>,
    settings: Vec<(String, String)>,
}

fn held(model: &Model) -> Vec<(String, Option<usize>, Vec<HeldPart>)> {
    model
        .objects()
        .iter()
        .map(|object| {
            let parts = object
                .shape()
                .parts()
                .iter()
                .map(|part| HeldPart {
                    name: part.name.clone(),
                    kind: part.kind,
                    filament: part.filament,
                    own_filament: part.own_filament,
                    transform: part.transform.rows.map(|row| row.map(f64::to_bits)),
                    mesh: bits(&part.mesh),
                    paint: part.paint.as_deref().cloned(),
                    seam: part.seam.as_deref().cloned(),
                    settings: part.settings(),
                })
                .collect();
            (object.name().to_owned(), object.filament(), parts)
        })
        .collect()
}

/// A plate's G-code, or why there is none.
fn sliced(model: &Model, plate: Option<u32>) -> Result<Vec<u8>, String> {
    let job = edge_engine::prepare(
        model.place(plate).map_err(|e| e.to_string())?,
        &Chosen::default(),
        &Overrides::default(),
    )
    .map_err(|e| e.to_string())?;
    let sliced = edge_engine::slice(job, &Unwatched).map_err(|e| e.to_string())?;
    Ok(sliced
        .gcode("2001-01-01 at 00:00:00")
        .map_err(|e| e.to_string())?
        .text)
}

/// Saves an edited model, opens the file again and checks it is the edited
/// model: its parts as they stand, and the same G-code on `plates`. Nothing
/// is asked about first, and saved twice it is the same bytes. With
/// `EDGE_KEEP_SAVED` set to a folder, the file is copied there as `name`
/// (to give to the reference). Answers the model read back and the file's
/// `model_settings.config`, followed by its objects' files.
fn saved_as_edited(model: &Model, name: &str, plates: &[Option<u32>]) -> (Model, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(name);
    let saving = as_opened(model);
    assert!(model.save_losses(&saving).is_empty(), "{name}: losses");
    model.save(&file, &saving, Some(&U1)).unwrap();
    let again = Model::open(&file).unwrap();
    assert_eq!(held(model), held(&again), "{name}: parts");
    for plate in plates {
        let (a, b) = (sliced(model, *plate), sliced(&again, *plate));
        assert!(a == b, "{name}: plate {plate:?}");
    }
    let twice = dir.path().join("twice.3mf");
    model.save(&twice, &saving, Some(&U1)).unwrap();
    assert!(
        std::fs::read(&file).unwrap() == std::fs::read(&twice).unwrap(),
        "{name}: saved twice"
    );
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        std::fs::copy(&file, Path::new(&keep).join(name)).unwrap();
    }
    let settings = entry(&file, "Metadata/model_settings.config");
    let objects: String = entries(&file)
        .iter()
        .filter(|name| name.starts_with("3D/Objects/"))
        .map(|name| entry(&file, name))
        .collect();
    (again, settings + &objects)
}

fn twopart() -> Model {
    Model::open(&reference("tests/data/support_corpus/twopart_groups.3mf")).unwrap()
}

/// The first object's id and its parts' ids.
fn parts_of(model: &Model) -> (u64, Vec<u64>) {
    let object = &model.objects()[0];
    (object.id(), object.shape().part_ids().to_vec())
}

/// `<part>` entries of `model_settings.config`, in order.
fn part_entries(settings: &str) -> Vec<&str> {
    settings
        .split("<part ")
        .skip(1)
        .map(|rest| &rest[..rest.find("</part>").unwrap()])
        .collect()
}

const ONE: [Option<u32>; 1] = [Some(1)];

#[test]
fn a_parts_filament_saved_is_its_own_and_default_is_none() {
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    let bed = Some(&U1);
    model
        .set_part_filament(bed, object, &[parts[0]], Some(2))
        .unwrap();
    let (again, settings) = saved_as_edited(&model, "part_filament.3mf", &ONE);
    assert!(part_entries(&settings)[0].contains(r#"<metadata key="extruder" value="2"/>"#));
    // Put back to "default", it follows its object again.
    let mut model = again;
    let (object, parts) = parts_of(&model);
    model
        .set_part_filament(bed, object, &[parts[0]], None)
        .unwrap();
    let (again, settings) = saved_as_edited(&model, "part_default.3mf", &ONE);
    assert!(!part_entries(&settings)[0].contains("extruder"));
    assert_eq!(again.objects()[0].shape().parts()[0].own_filament, None);
}

#[test]
fn an_objects_filament_saved_clears_its_parts_own() {
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    let bed = Some(&U1);
    model
        .set_part_filament(bed, object, &[parts[1]], Some(1))
        .unwrap();
    model.set_filament(bed, object, 2).unwrap();
    let (again, settings) = saved_as_edited(&model, "object_filament.3mf", &ONE);
    assert_eq!(again.objects()[0].filament(), Some(2));
    let object_entries = &settings[..settings.find("<part ").unwrap()];
    assert!(object_entries.contains(r#"<metadata key="extruder" value="2"/>"#));
    assert!(
        part_entries(&settings)
            .iter()
            .all(|part| !part.contains("extruder"))
    );
}

#[test]
fn a_part_renamed_is_saved_with_its_name_in_place() {
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    model
        .rename_part(Some(&U1), object, parts[1], "Bridge & <beam>")
        .unwrap();
    let (_, settings) = saved_as_edited(&model, "part_renamed.3mf", &ONE);
    let second = part_entries(&settings)[1];
    // Where the file had it: after the part's settings.
    let name = second
        .find(r#"<metadata key="name" value="Bridge &amp; &lt;beam&gt;"/>"#)
        .unwrap();
    assert!(second.find("support_group").unwrap() < name);
    assert!(name < second.find("matrix").unwrap());
}

#[test]
fn parts_deleted_take_their_entries_and_the_last_gives_its_settings_to_the_object() {
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    let bed = Some(&U1);
    model.remove_parts(bed, object, &[parts[0]]).unwrap();
    let (again, settings) = saved_as_edited(&model, "part_deleted.3mf", &ONE);
    let entries = part_entries(&settings);
    assert_eq!(entries.len(), 2);
    // The part with settings is now the first, and keeps them.
    assert!(entries[0].contains("support_group"));
    assert_eq!(again.objects()[0].shape().parts().len(), 2);

    // One part left: the reference gives its settings to the object.
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    model
        .remove_parts(bed, object, &[parts[0], parts[2]])
        .unwrap();
    let (again, settings) = saved_as_edited(&model, "one_part_left.3mf", &ONE);
    let object_entries = &settings[..settings.find("<part ").unwrap()];
    assert!(object_entries.contains(r#"<metadata key="support_group" value="B"/>"#));
    assert!(!part_entries(&settings)[0].contains("support_group"));
    assert!(again.objects()[0].shape().parts()[0].settings().is_empty());
}

fn object_of(model: &Model) -> u64 {
    model.objects()[0].id()
}

/// A turn about Z by `degrees`, then a move.
fn turned(degrees: f64, then: [f64; 3]) -> Transform {
    let (s, c) = degrees.to_radians().sin_cos();
    Transform {
        rows: [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0], then],
    }
}

#[test]
fn parts_moved_turned_scaled_and_mirrored_are_saved_where_they_stand() {
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    let bed = Some(&U1);
    let shape = model.objects()[0].shape().clone();
    let moved = shape.parts()[2]
        .transform
        .then(&Transform::translation([4.0, 3.0, 0.0]));
    let turned = shape.parts()[2].transform.then(&turned(30.0, [0.0; 3]));
    let scaled = Transform::scaling(1.5).then(&shape.parts()[1].transform);
    model
        .set_part_transforms(bed, object, &[(parts[2], moved)])
        .unwrap();
    let (_, settings) = saved_as_edited(&model, "part_moved.3mf", &ONE);
    // The matrix says where it now stands; the others' are as they were.
    let entries = part_entries(&settings);
    assert!(
        entries[2].contains(r#"value="1 0 0 11 0 1 0 3 0 0 1 11 0 0 0 1""#),
        "{}",
        entries[2]
    );
    assert!(entries[1].contains(r#"value="1 0 0 -10 0 1 0 0 0 0 1 11 0 0 0 1""#));

    model
        .set_part_transforms(bed, object, &[(parts[2], turned), (parts[1], scaled)])
        .unwrap();
    saved_as_edited(&model, "part_turned_scaled.3mf", &ONE);

    let copy = model.objects()[0].copies()[0].id();
    model.mirror_parts(bed, copy, &[parts[1]], 0).unwrap();
    assert!(
        model.objects()[0].shape().parts()[1]
            .transform
            .is_mirrored()
    );
    saved_as_edited(&model, "part_mirrored.3mf", &ONE);
}

#[test]
fn a_parts_kind_changed_is_saved_as_its_subtype() {
    use edge_engine::PartKind;
    for (kind, subtype) in [
        (PartKind::Modifier, "modifier_part"),
        (PartKind::Negative, "negative_part"),
        (PartKind::SupportBlocker, "support_blocker"),
        (PartKind::SupportEnforcer, "support_enforcer"),
    ] {
        let mut model = twopart();
        let (object, parts) = parts_of(&model);
        model
            .set_part_kind(Some(&U1), object, &[parts[2]], kind)
            .unwrap();
        let (_, settings) = saved_as_edited(&model, &format!("part_{subtype}.3mf"), &ONE);
        assert!(
            settings.contains(&format!(r#"subtype="{subtype}""#)),
            "{subtype}"
        );
    }
    // A Precise Seam part is a modifier naming its type, its settings
    // dormant under a prefix; made a part again, they are its own again.
    let mut model = twopart();
    let (object, parts) = parts_of(&model);
    model
        .set_part_kind(Some(&U1), object, &[parts[1]], PartKind::PreciseSeam)
        .unwrap();
    let (again, settings) = saved_as_edited(&model, "part_precise_seam.3mf", &ONE);
    let second = part_entries(&settings)[1];
    assert!(
        second.starts_with(r#"id="2" subtype="modifier_part">"#),
        "{second}"
    );
    assert!(second.contains(r#"<metadata key="precise_seam_type" value="precise_seam_center"/>"#));
    assert!(second.contains(r#"key="precise_seam_config:support_group""#));
    let mut model = again;
    let (object, parts) = parts_of(&model);
    model
        .set_part_kind(Some(&U1), object, &[parts[1]], PartKind::Normal)
        .unwrap();
    let (_, settings) = saved_as_edited(&model, "part_seam_undone.3mf", &ONE);
    let second = part_entries(&settings)[1];
    assert!(
        second.starts_with(r#"id="2" subtype="normal_part">"#),
        "{second}"
    );
    assert!(!second.contains("precise_seam"));
    assert!(second.contains(r#"<metadata key="support_group" value="B"/>"#));
}

#[test]
fn parts_added_are_saved_with_their_name_kind_and_place() {
    use edge_engine::PartKind;
    let dir = tempfile::tempdir().unwrap();
    let mut model = twopart();
    let object = object_of(&model);
    model
        .add_generic(Some(&U1), object, None, PartKind::Modifier, "Cube")
        .unwrap();
    let mesh = Mesh::from_triangle_soup(edge_mesh::primitives::cuboid(
        [0.0, 0.0, 0.0],
        [4.0, 4.0, 4.0],
    ));
    let stl = dir.path().join("peg.stl");
    std::fs::write(&stl, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let other = Model::open(&stl).unwrap();
    model
        .add_from(Some(&U1), object, &other, PartKind::Normal)
        .unwrap();
    let (_, settings) = saved_as_edited(&model, "parts_added.3mf", &ONE);
    let entries = part_entries(&settings);
    assert_eq!(entries.len(), 5);
    assert!(entries[3].contains(r#"subtype="modifier_part""#));
    assert!(entries[3].contains(r#"<metadata key="name" value="Generic-Cube"/>"#));
    assert!(entries[4].contains(r#"<metadata key="name" value="peg.stl"/>"#));
}

/// A project of one object of two parts, laid out for the U1 with
/// `twopart_groups.3mf`'s settings: "Pair", two 8 mm cubes 4 mm apart as
/// one mesh, its first triangle painted in filament 2, with settings of
/// its own; then "Lid", a cube in filament 2 of its own.
fn pair(dir: &Path) -> PathBuf {
    let cube = |at: [f32; 3], size: f32| {
        edge_mesh::primitives::cuboid(at, [at[0] + size, at[1] + size, at[2] + size])
    };
    let mesh_xml = |mesh: &Mesh, paint: bool| {
        let mut text = String::from("<mesh><vertices>");
        for [x, y, z] in &mesh.vertices {
            text += &format!(r#"<vertex x="{x}" y="{y}" z="{z}"/>"#);
        }
        text += "</vertices><triangles>";
        for (at, [a, b, c]) in mesh.triangles.iter().enumerate() {
            let painted = if paint && at == 0 {
                r#" paint_color="8""#
            } else {
                ""
            };
            text += &format!(r#"<triangle v1="{a}" v2="{b}" v3="{c}"{painted}/>"#);
        }
        text + "</triangles></mesh>"
    };
    let pair = Mesh::from_triangle_soup(
        cube([0.0; 3], 8.0)
            .into_iter()
            .chain(cube([12.0, 0.0, 0.0], 8.0)),
    );
    let lid = Mesh::from_triangle_soup(cube([0.0, 12.0, 0.0], 8.0));
    let model = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<model unit="millimeter" xml:lang="en-US" xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" xmlns:BambuStudio="http://schemas.bambulab.com/package/2021">
 <metadata name="Application">EdgeSlicer-2.4.9.0</metadata>
 <metadata name="BambuStudio:3mfVersion">1</metadata>
 <resources>
  <object id="1" type="model">{}</object>
  <object id="2" type="model">{}</object>
  <object id="3" type="model"><components>
   <component objectid="1" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
   <component objectid="2" transform="1 0 0 0 1 0 0 0 1 0 0 0"/>
  </components></object>
 </resources>
 <build><item objectid="3" transform="1 0 0 0 1 0 0 0 1 125 125 0" printable="1"/></build>
</model>"#,
        mesh_xml(&pair, true),
        mesh_xml(&lid, false)
    );
    let settings = r#"<?xml version="1.0" encoding="UTF-8"?>
<config>
  <object id="3">
    <metadata key="name" value="Pair"/>
    <metadata key="extruder" value="1"/>
    <part id="1" subtype="normal_part">
      <metadata key="name" value="Pair"/>
      <metadata key="matrix" value="1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1"/>
      <metadata key="source_file" value="pair.stl"/>
      <metadata key="source_object_id" value="0"/>
      <metadata key="source_volume_id" value="0"/>
      <metadata key="wall_loops" value="3"/>
      <mesh_stat edges_fixed="0" degenerate_facets="0" facets_removed="0" facets_reversed="0" backwards_edges="0"/>
    </part>
    <part id="2" subtype="normal_part">
      <metadata key="name" value="Lid"/>
      <metadata key="matrix" value="1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 1"/>
      <metadata key="extruder" value="2"/>
      <mesh_stat edges_fixed="0" degenerate_facets="0" facets_removed="0" facets_reversed="0" backwards_edges="0"/>
    </part>
  </object>
  <plate>
    <metadata key="plater_id" value="1"/>
    <metadata key="plater_name" value=""/>
    <metadata key="locked" value="false"/>
    <model_instance>
      <metadata key="object_id" value="3"/>
      <metadata key="instance_id" value="0"/>
      <metadata key="identify_id" value="7"/>
    </model_instance>
  </plate>
</config>
"#;
    let project_settings = entry(
        &reference("tests/data/support_corpus/twopart_groups.3mf"),
        "Metadata/project_settings.config",
    );
    let path = dir.join("pair.3mf");
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    let options = zip::write::SimpleFileOptions::default();
    for (name, text) in [
        ("[Content_Types].xml", r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#.to_owned()),
        ("_rels/.rels", r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#.to_owned()),
        ("3D/3dmodel.model", model),
        ("Metadata/model_settings.config", settings.to_owned()),
        ("Metadata/project_settings.config", project_settings),
    ] {
        use std::io::Write;
        zip.start_file(name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    path
}

#[test]
fn a_part_split_saves_its_pieces_with_its_settings_and_without_its_paint() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::open(&pair(dir.path())).unwrap();
    let (object, parts) = parts_of(&model);
    assert!(model.objects()[0].shape().parts()[0].paint.is_some());
    // Unedited, the paint and the settings are saved as they were.
    let (_, settings) = saved_as_edited(&model, "pair.3mf", &ONE);
    assert!(part_entries(&settings)[0].contains("wall_loops"));
    assert!(settings.contains(r#"paint_color="8""#));
    model.split_part(Some(&U1), object, parts[0]).unwrap();
    let shape = model.objects()[0].shape().clone();
    assert_eq!(shape.parts().len(), 3);
    let (again, settings) = saved_as_edited(&model, "pair_split.3mf", &ONE);
    let entries = part_entries(&settings);
    for piece in &entries[..2] {
        // Each piece keeps the part's settings and filament, as a copy of
        // it; not where it came from, nor the paint.
        assert!(
            piece.contains(r#"<metadata key="wall_loops" value="3"/>"#),
            "{piece}"
        );
        assert!(
            piece.contains(r#"<metadata key="extruder" value="1"/>"#),
            "{piece}"
        );
        assert!(!piece.contains("source_"), "{piece}");
    }
    assert!(entries[0].contains(r#"value="Pair_1""#));
    assert!(again.objects()[0].shape().parts()[0].paint.is_none());
    assert!(!settings.contains("paint_color"));
}

#[test]
fn a_part_put_back_to_default_follows_its_object_and_one_deleted_leaves_the_others_paint() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::open(&pair(dir.path())).unwrap();
    let (object, parts) = parts_of(&model);
    // "Lid" put back to default: it prints in the object's filament.
    model
        .set_part_filament(Some(&U1), object, &[parts[1]], None)
        .unwrap();
    let (_, settings) = saved_as_edited(&model, "pair_default.3mf", &ONE);
    assert!(!part_entries(&settings)[1].contains("extruder"));
    // "Lid" deleted: the paint stays on "Pair", whose settings go to the
    // object, the one part left.
    let mut model = Model::open(&pair(dir.path())).unwrap();
    let (object, parts) = parts_of(&model);
    model.remove_parts(Some(&U1), object, &[parts[1]]).unwrap();
    let (again, settings) = saved_as_edited(&model, "pair_lid_deleted.3mf", &ONE);
    assert!(again.objects()[0].shape().parts()[0].paint.is_some());
    let object_entries = &settings[..settings.find("<part ").unwrap()];
    assert!(object_entries.contains(r#"<metadata key="wall_loops" value="3"/>"#));
}

#[test]
fn a_painted_projects_part_deleted_and_given_a_filament_keeps_the_rest_of_its_paint() {
    let mut model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let bed = Some(&U1);
    let at = model
        .objects()
        .iter()
        .position(|object| object.parts() == 3)
        .unwrap();
    let object = model.objects()[at].id();
    let parts = model.objects()[at].shape().part_ids().to_vec();
    let copy = model.objects()[at].copies()[0].id();
    let plate = model
        .plates()
        .iter()
        .find(|plate| plate.copies().contains(&copy))
        .map(|plate| plate.id());
    model.remove_parts(bed, object, &[parts[0]]).unwrap();
    model
        .set_part_filament(bed, object, &[parts[2]], Some(4))
        .unwrap();
    let (again, _) = saved_as_edited(&model, "mcbrim_parts.3mf", &[plate]);
    let shape = again.objects()[at].shape();
    assert_eq!(shape.parts().len(), 2);
    assert!(shape.parts().iter().all(|part| part.paint.is_some()));
}
