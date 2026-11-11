//! A model's plates as plates: named, locked, given settings of their own,
//! moved to the front and duplicated; each kept by a save and read back,
//! and each followed by a slice as the reference follows it.

use std::{
    io::Read,
    path::{Path, PathBuf},
};

use edge_engine::{
    Bed, Chosen, LAST_LAYER, LayerSequence, Model, Overrides, PlateSettings, Saving,
};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// The Snapmaker U1's bed, which `mcbrim_test.3mf` is laid out for.
const U1: Bed = Bed {
    min: [0.5, 1.0],
    max: [270.5, 271.0],
};

fn mcbrim() -> Model {
    Model::open(&reference("tests/mcbrim_test.3mf")).unwrap()
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

/// Saves a model with its own settings and opens the file again; saved
/// twice, the same bytes. Answers the model read back and the file's
/// `model_settings.config`.
fn round_trip(model: &Model, dir: &Path, name: &str) -> (Model, String) {
    let path = dir.join(name);
    model.save(&path, &Saving::Own, Some(&U1)).unwrap();
    let twice = dir.join(format!("twice_{name}"));
    model.save(&twice, &Saving::Own, Some(&U1)).unwrap();
    assert!(
        std::fs::read(&path).unwrap() == std::fs::read(&twice).unwrap(),
        "{name}: saved twice"
    );
    if let Some(keep) = std::env::var_os("EDGE_KEEP_SAVED") {
        std::fs::copy(&path, Path::new(&keep).join(name)).unwrap();
    }
    (
        Model::open(&path).unwrap(),
        entry(&path, "Metadata/model_settings.config"),
    )
}

/// Each plate's name, lock and own settings.
fn plates_of(model: &Model) -> Vec<(u32, String, bool, PlateSettings)> {
    model
        .plates()
        .iter()
        .map(|plate| {
            (
                plate.id(),
                plate.name().to_owned(),
                plate.locked(),
                plate.settings(),
            )
        })
        .collect()
}

/// Each copy on a plate by its object's name, where it stands on its plate,
/// to a ten-thousandth of a millimetre: a plate laid out at another place
/// in the grid and read back may be a bit of an `f32` away.
fn standing(model: &Model, plate: u32) -> Vec<(String, [i64; 6])> {
    let loaded = model.place(Some(plate)).unwrap();
    let mut names: Vec<String> = model
        .plate_list()
        .iter()
        .find(|listed| listed.id == plate)
        .map(|listed| listed.objects.clone())
        .unwrap_or_default();
    names.sort();
    let mut boxes: Vec<[i64; 6]> = loaded
        .objects
        .iter()
        .map(|mesh| {
            let (low, high) = mesh.bounds().unwrap();
            [low[0], low[1], low[2], high[0], high[1], high[2]]
                .map(|v| (f64::from(v) * 1e4).round() as i64)
        })
        .collect();
    boxes.sort();
    names.into_iter().zip(boxes).collect()
}

/// Two plates' [`standing`] alike, but for a ten-thousandth of a
/// millimetre either way.
fn alike(a: &[(String, [i64; 6])], b: &[(String, [i64; 6])]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|((one, x), (other, y))| {
            one == other && x.iter().zip(y).all(|(x, y)| (x - y).abs() <= 1)
        })
}

#[test]
fn a_plate_renamed_locked_and_customized_is_saved_and_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = mcbrim();
    // Nothing done: every plate as the file has it.
    assert!(plates_of(&model).iter().all(|(_, _, locked, own)| {
        !locked
            && own.bed_type.is_none()
            && own.print_sequence.is_none()
            && own.spiral_mode.is_none()
            && own.other_layers_sequence.is_empty()
    }));
    model.rename_plate(2, "Brims & <rims>").unwrap();
    model.lock_plate(2, true).unwrap();
    let own = PlateSettings {
        bed_type: Some("Textured PEI Plate".into()),
        print_sequence: Some("by object".into()),
        spiral_mode: Some(false),
        first_layer_sequence: vec![3, 1, 2, 4, 5, 6, 7],
        other_layers_sequence: vec![
            LayerSequence {
                begin: 10,
                end: LAST_LAYER,
                order: vec![2, 1],
            },
            LayerSequence {
                begin: 2,
                end: 9,
                order: vec![1, 2],
            },
        ],
    };
    model.set_plate_settings(3, &own).unwrap();
    let (again, settings) = round_trip(&model, dir.path(), "customized.3mf");
    assert_eq!(plates_of(&again), plates_of(&model));
    let plates = plates_of(&again);
    assert_eq!(plates[1].1, "Brims & <rims>");
    assert!(plates[1].2);
    // The ranges in order of their first layer, as the reference sorts them.
    assert_eq!(plates[2].3.other_layers_sequence[0].begin, 2);
    // Written in the reference's keys and order.
    let third = settings.split("<plate>").nth(3).unwrap();
    let keys: Vec<&str> = third
        .lines()
        .filter_map(|line| line.split("key=\"").nth(1)?.split('"').next())
        .collect();
    assert_eq!(
        &keys[..10],
        [
            "plater_id",
            "plater_name",
            "locked",
            "bed_type",
            "print_sequence",
            "first_layer_print_sequence",
            "other_layers_print_sequence",
            "other_layers_print_sequence_nums",
            "spiral_mode",
            "filament_map_mode",
        ]
    );
    assert!(third.contains(r#"key="bed_type" value="Textured PEI Plate""#));
    assert!(
        third.contains(r#"key="other_layers_print_sequence" value="2 9 1 2 10 2147483646 2 1""#)
    );
    assert!(third.contains(r#"key="spiral_mode" value="false""#));
    assert!(settings.contains(r#"key="plater_name" value="Brims &amp; &lt;rims&gt;""#));
    assert!(
        settings
            .split("<plate>")
            .nth(2)
            .unwrap()
            .contains(r#"key="locked" value="true""#)
    );

    // Back to the project's: the plate's entries are the file's again.
    let mut back = model.clone();
    back.set_plate_settings(
        3,
        &PlateSettings {
            first_layer_sequence: Vec::new(),
            ..PlateSettings::default()
        },
    )
    .unwrap();
    back.rename_plate(2, "").unwrap();
    back.lock_plate(2, false).unwrap();
    let (_, plain) = round_trip(&back, dir.path(), "plain.3mf");
    let (_, unedited) = round_trip(&mcbrim(), dir.path(), "unedited.3mf");
    assert_eq!(plain, unedited);
}

#[test]
fn a_plate_that_was_not_in_the_file_saves_its_settings_too() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = mcbrim();
    let added = model.add_plate(Some(&U1)).unwrap();
    // Nothing set: written as a new plate always was.
    let (_, before) = round_trip(&model, dir.path(), "added.3mf");
    model
        .set_plate_settings(added, &PlateSettings::default())
        .unwrap();
    let (_, unchanged) = round_trip(&model, dir.path(), "added_again.3mf");
    assert_eq!(before, unchanged);
    model.lock_plate(added, true).unwrap();
    model
        .set_plate_settings(
            added,
            &PlateSettings {
                spiral_mode: Some(true),
                ..PlateSettings::default()
            },
        )
        .unwrap();
    let (again, _) = round_trip(&model, dir.path(), "added_set.3mf");
    let plate = again.plates().last().unwrap();
    assert!(plate.locked());
    assert_eq!(plate.settings().spiral_mode, Some(true));
}

#[test]
fn what_a_plate_cannot_be_set_to_is_refused() {
    let mut model = mcbrim();
    let refused = |model: &mut Model, own: PlateSettings| {
        model.set_plate_settings(1, &own).unwrap_err().to_string()
    };
    assert!(
        refused(
            &mut model,
            PlateSettings {
                bed_type: Some("Glass".into()),
                ..PlateSettings::default()
            }
        )
        .contains("Glass")
    );
    assert_eq!(
        refused(
            &mut model,
            PlateSettings {
                other_layers_sequence: vec![LayerSequence {
                    begin: 1,
                    end: 5,
                    order: vec![1],
                }],
                ..PlateSettings::default()
            }
        ),
        "Please input layer value (>= 2)."
    );
    assert!(
        model
            .set_plate_settings(
                1,
                &PlateSettings {
                    first_layer_sequence: vec![1, 1],
                    ..PlateSettings::default()
                }
            )
            .is_err()
    );
    assert!(model.rename_plate(1, &"x".repeat(251)).is_err());
    assert!(model.rename_plate(9, "nine").is_err());
    // Nothing changed.
    assert_eq!(plates_of(&model), plates_of(&mcbrim()));
}

#[test]
fn a_locked_plate_is_left_alone_by_arrange() {
    let mut model = mcbrim();
    model.lock_plate(1, true).unwrap();
    let before = standing(&model, 1);
    let others: Vec<_> = [2, 3].map(|plate| standing(&model, plate)).into();
    model.arrange(Some(&U1), None).unwrap();
    assert_eq!(standing(&model, 1), before);
    // Arranged alone, it is refused in the reference's words.
    assert_eq!(
        model.arrange(Some(&U1), Some(1)).unwrap_err().to_string(),
        "This plate is locked.\nCannot auto-arrange on this plate."
    );
    // The others were arranged (plate 2's copies stand elsewhere now).
    let after: Vec<_> = [2, 3].map(|plate| standing(&model, plate)).into();
    assert_ne!(after, others);
    // Unlocked, it is arranged.
    model.lock_plate(1, false).unwrap();
    model.arrange(Some(&U1), Some(1)).unwrap();
    assert_ne!(standing(&model, 1), before);
    // A plate left alone by the session is too.
    let mut model = mcbrim();
    let before = standing(&model, 2);
    model.arrange_leaving(Some(&U1), None, &[2]).unwrap();
    assert_eq!(standing(&model, 2), before);
}

#[test]
fn a_plate_moved_to_the_front_takes_what_stands_on_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = mcbrim();
    model.rename_plate(3, "third").unwrap();
    let (third, first, second) = (
        standing(&model, 3),
        standing(&model, 1),
        standing(&model, 2),
    );
    let sequence = model.plates()[2].settings().first_layer_sequence;
    model.move_plate_to_front(Some(&U1), 3).unwrap();
    let ids: Vec<u32> = model.plates().iter().map(|plate| plate.id()).collect();
    assert_eq!(ids, [1, 2, 3]);
    assert_eq!(model.plates()[0].name(), "third");
    assert_eq!(standing(&model, 1), third);
    assert_eq!(standing(&model, 2), first);
    assert_eq!(standing(&model, 3), second);
    assert_eq!(model.plates()[0].settings().first_layer_sequence, sequence);
    assert!(
        model
            .move_plate_to_front(Some(&U1), 1)
            .unwrap_err()
            .to_string()
            .contains("front already")
    );
    let (again, _) = round_trip(&model, dir.path(), "front.3mf");
    assert_eq!(plates_of(&again), plates_of(&model));
    for plate in 1..=3 {
        assert!(alike(&standing(&again, plate), &standing(&model, plate)));
    }
}

#[test]
fn a_plate_duplicated_is_a_new_plate_with_the_same_objects_where_they_stood() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = mcbrim();
    model.rename_plate(2, "second").unwrap();
    model.lock_plate(2, true).unwrap();
    let objects = model.objects().len();
    let (id, made) = model.duplicate_plate(Some(&U1), 2).unwrap();
    assert_eq!(id, 4);
    assert_eq!(made.len(), model.plates()[1].copies().len());
    assert_eq!(model.objects().len(), objects + made.len());
    assert!(alike(&standing(&model, 4), &standing(&model, 2)));
    // The plate is new: no name, no lock, nothing of its own.
    let new = &model.plates()[3];
    assert_eq!(new.name(), "");
    assert!(!new.locked());
    let (again, _) = round_trip(&model, dir.path(), "duplicated.3mf");
    assert!(alike(&standing(&again, 4), &standing(&model, 4)));
    assert_eq!(plates_of(&again), plates_of(&model));
}

#[test]
fn a_plates_own_settings_reach_the_slice() {
    let mut model = mcbrim();
    let settled = |model: &Model, plate: u32| {
        edge_engine::prepare(
            model.place(Some(plate)).unwrap(),
            &Chosen::default(),
            &Overrides::default(),
        )
        .unwrap()
        .settings
    };
    let project = settled(&model, 1);
    assert_eq!(project.text("curr_bed_type"), Some("Textured PEI Plate"));
    let stamp = model.stamp(Some(1));
    model
        .set_plate_settings(
            1,
            &PlateSettings {
                bed_type: Some("Cool Plate".into()),
                print_sequence: Some("by object".into()),
                spiral_mode: Some(true),
                first_layer_sequence: model.plates()[0].settings().first_layer_sequence,
                ..PlateSettings::default()
            },
        )
        .unwrap();
    // The plate's slice depends on them; the others' do not.
    assert_ne!(model.stamp(Some(1)), stamp);
    let own = settled(&model, 1);
    assert_eq!(own.text("curr_bed_type"), Some("Cool Plate"));
    assert_eq!(own.text("print_sequence"), Some("by object"));
    assert_eq!(own.text("spiral_mode"), Some("1"));
    // Plate 2 is the project's.
    assert_eq!(
        settled(&model, 2).text("curr_bed_type"),
        project.text("curr_bed_type")
    );
    // A name and a lock are not what a slice depends on.
    let stamp = model.stamp(Some(2));
    model.rename_plate(2, "named").unwrap();
    model.lock_plate(2, true).unwrap();
    assert_eq!(model.stamp(Some(2)), stamp);
}

#[test]
fn bed_types_another_printer_does_not_take_are_reset() {
    let mut model = mcbrim();
    for (plate, bed) in [(1, "Textured PEI Plate"), (2, "Engineering Plate")] {
        model
            .set_plate_settings(
                plate,
                &PlateSettings {
                    bed_type: Some(bed.into()),
                    first_layer_sequence: model.plates()[plate as usize - 1]
                        .settings()
                        .first_layer_sequence,
                    ..PlateSettings::default()
                },
            )
            .unwrap();
    }
    assert!(!model.reset_bed_types(|_| true));
    assert!(model.reset_bed_types(|bed| bed == "Textured PEI Plate"));
    let beds: Vec<Option<String>> = model
        .plates()
        .iter()
        .map(|plate| plate.settings().bed_type)
        .collect();
    assert_eq!(beds, [Some("Textured PEI Plate".into()), None, None]);
}
