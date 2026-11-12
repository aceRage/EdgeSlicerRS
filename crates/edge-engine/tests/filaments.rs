//! The project's filaments as a list: one added, one taken away or merged
//! into another, a colour changed; what printed in each renumbered (parts,
//! objects, painted colours, plates' orders), the settings laid out for
//! the list, and all of it kept through a save.

use std::path::{Path, PathBuf};

use edge_engine::{Bed, Chosen, FlushBasis, Model, Overrides, Saving, Unwatched};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

const U1: Bed = Bed {
    min: [0.5, 1.0],
    max: [270.5, 271.0],
};

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

/// A part's filament, its own, and each painted piece's.
type PartFilaments = (usize, Option<usize>, Vec<u8>);

/// Every part's filament and every painted triangle's, object by object.
fn filaments(model: &Model) -> Vec<(Option<usize>, Vec<PartFilaments>)> {
    model
        .objects()
        .iter()
        .map(|object| {
            let parts = object
                .shape()
                .parts()
                .iter()
                .map(|part| {
                    let paint = part
                        .paint
                        .as_deref()
                        .map(|paint| paint.iter().map(|facet| facet.filament).collect())
                        .unwrap_or_default();
                    (part.filament, part.own_filament, paint)
                })
                .collect();
            (object.filament(), parts)
        })
        .collect()
}

fn texts(model: &Model, key: &str) -> Vec<String> {
    model.settings().unwrap().texts(key).unwrap().to_vec()
}

#[test]
fn a_filament_taken_away_renumbers_what_printed_in_those_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let opened = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    assert_eq!(opened.filament_count(), 7);
    let was = filaments(&opened);
    let painted = was
        .iter()
        .flat_map(|(_, parts)| parts)
        .any(|(_, _, paint)| paint.iter().any(|filament| *filament > 3));
    assert!(painted, "the project paints in filaments after 3");
    let colours = opened.filament_colours();
    let names = texts(&opened, "filament_settings_id");
    let temperatures = texts(&opened, "nozzle_temperature");

    let mut model = opened.clone();
    model.remove_filament(Some(&U1), 3, None).unwrap();
    assert_eq!(model.filament_count(), 6);
    let map = |old: usize| match old {
        3 => 0,
        f if f > 3 => f - 1,
        f => f,
    };
    // Parts on 3 print in their object's; painted 3 is unpainted.
    let expected: Vec<_> = was
        .iter()
        .map(|(object, parts)| {
            let object = object.map(map).filter(|f| *f > 0);
            let parts = parts
                .iter()
                .map(|(_, own, paint)| {
                    let own = own.map(map).filter(|f| *f > 0);
                    let paint: Vec<u8> = paint
                        .iter()
                        .map(|f| {
                            if *f == 0 {
                                0
                            } else {
                                map(usize::from(*f)) as u8
                            }
                        })
                        .collect();
                    // All of it unpainted is no paint.
                    let paint = if paint.iter().all(|f| *f == 0) {
                        Vec::new()
                    } else {
                        paint
                    };
                    (own.unwrap_or(object.unwrap_or(1)), own, paint)
                })
                .collect();
            (object, parts)
        })
        .collect();
    assert_eq!(filaments(&model), expected);
    let gone = |list: &[String]| {
        let mut list = list.to_vec();
        list.remove(2);
        list
    };
    assert_eq!(model.filament_colours(), gone(&colours));
    assert_eq!(texts(&model, "filament_settings_id"), gone(&names));
    assert_eq!(texts(&model, "nozzle_temperature"), gone(&temperatures));
    assert_eq!(texts(&model, "flush_volumes_matrix").len(), 36);
    assert_eq!(texts(&model, "flush_volumes_vector").len(), 12);
    // The plates list the filaments renumbered.
    for (now, before) in model.plate_list().iter().zip(opened.plate_list()) {
        let mut renumbered: Vec<usize> = before
            .filaments
            .iter()
            .map(|f| map(*f))
            .map(|f| if f == 0 { 1 } else { f })
            .collect();
        renumbered.sort_unstable();
        renumbered.dedup();
        assert_eq!(now.filaments, renumbered, "plate {}", now.id);
    }

    // Saved and opened again: the same, and it slices the same.
    let path = dir.path().join("deleted.3mf");
    model.save(&path, &Saving::Own, Some(&U1)).unwrap();
    let again = Model::open(&path).unwrap();
    assert_eq!(again.filament_count(), 6);
    assert_eq!(filaments(&again), filaments(&model));
    assert_eq!(again.filament_colours(), model.filament_colours());
    for plate in [1, 2, 3] {
        assert!(
            gcode(&again, Some(plate)) == gcode(&model, Some(plate)),
            "plate {plate}"
        );
    }
}

#[test]
fn a_filament_merged_into_another_prints_in_it() {
    let opened = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let mut model = opened.clone();
    model.remove_filament(Some(&U1), 2, Some(6)).unwrap();
    for ((_, before), (_, after)) in filaments(&opened).iter().zip(&filaments(&model)) {
        for ((filament, _, paint), (now, _, painted)) in before.iter().zip(after) {
            let map = |old: usize| match old {
                2 => 5,
                f if f > 2 => f - 1,
                f => f,
            };
            assert_eq!(*now, map(*filament));
            let expected: Vec<u8> = paint
                .iter()
                .map(|f| {
                    if *f == 0 {
                        0
                    } else {
                        map(usize::from(*f)) as u8
                    }
                })
                .collect();
            assert_eq!(*painted, expected);
        }
    }
    assert!(model.remove_filament(Some(&U1), 2, Some(2)).is_err());
}

#[test]
fn a_filament_added_is_the_last_again_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let opened = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let mut model = opened.clone();
    let basis = FlushBasis {
        minimums: vec![0; 8],
        support: vec![false; 8],
    };
    assert_eq!(model.add_filament(Some(&U1), "#00C1AE", &basis).unwrap(), 8);
    let names = texts(&model, "filament_settings_id");
    assert_eq!(names.len(), 8);
    assert_eq!(names[7], names[6]);
    assert_eq!(model.filament_colours()[7], "#00C1AE");
    let (matrix, vector) = model.flush_volumes();
    assert_eq!((matrix.len(), vector.len()), (64, 16));
    assert_eq!(matrix[7 * 8 + 7], 0.0);
    // Nothing is printed in it: every plate slices as before but for the
    // lists of filaments its settings name.
    assert_eq!(filaments(&model), filaments(&opened));
    let path = dir.path().join("added.3mf");
    model.save(&path, &Saving::Own, Some(&U1)).unwrap();
    let again = Model::open(&path).unwrap();
    assert_eq!(again.filament_count(), 8);
    assert_eq!(again.flush_volumes(), model.flush_volumes());
    assert!(gcode(&again, Some(1)) == gcode(&model, Some(1)));
}

#[test]
fn a_model_without_settings_is_given_the_lists_of_its_filaments() {
    let mut model =
        Model::open(&reference("resources/handy_models/OrcaToleranceTest.stl")).unwrap();
    assert!(model.underlying().is_none());
    let basis = FlushBasis {
        minimums: vec![0, 0],
        support: vec![false, false],
    };
    model.add_filament(Some(&U1), "#000000", &basis).unwrap();
    // Not settings of its own: it has no printer.
    assert!(model.settings().is_none());
    assert_eq!(model.filament_colours(), ["#F2754E", "#000000"]);
    let (matrix, _) = model.flush_volumes();
    assert_eq!(matrix[0], 0.0);
    assert_eq!(matrix[3], 0.0);
    assert!(matrix[1] > 0.0 && matrix[2] > 0.0);
    model
        .set_filament_colour(Some(&U1), 2, "#FFFFFF", Some(&basis))
        .unwrap();
    assert_eq!(model.filament_colours()[1], "#FFFFFF");
    assert!(model.set_filament_colour(None, 3, "#FFFFFF", None).is_err());
    model.remove_filament(Some(&U1), 2, None).unwrap();
    assert_eq!(model.filament_count(), 1);
    assert!(model.remove_filament(Some(&U1), 1, None).is_err());
}

#[test]
fn the_flushing_volumes_set_are_kept_and_come_over_a_printers_own() {
    let dir = tempfile::tempdir().unwrap();
    let mut model = Model::open(&reference("tests/mcbrim_test.3mf")).unwrap();
    let count = model.filament_count();
    let (matrix, vector) = model.flush_volumes();
    let mut set = matrix.clone();
    // One pair doubled, and something on the diagonal, which is not kept.
    set[1] *= 2.0;
    set[count + 1] = 50.0;
    model.set_flush_volumes(Some(&U1), &set, 0.5).unwrap();
    let (now, kept) = model.flush_volumes();
    assert_eq!(now[1], matrix[1] * 2.0);
    assert_eq!(now[count + 1], 0.0);
    assert_eq!(kept, vector);
    assert_eq!(model.flush_multiplier(), Some(0.5));
    // What cannot be: the wrong number of volumes, a negative one, a
    // multiplier past 3.
    assert!(model.set_flush_volumes(None, &set[1..], 1.0).is_err());
    let mut negative = set.clone();
    negative[2] = -1.0;
    assert!(model.set_flush_volumes(None, &negative, 1.0).is_err());
    assert!(model.set_flush_volumes(None, &set, 3.5).is_err());
    // Kept through a save.
    let path = dir.path().join("flushed.3mf");
    model.save(&path, &Saving::Own, Some(&U1)).unwrap();
    let again = Model::open(&path).unwrap();
    assert_eq!(again.flush_volumes(), model.flush_volumes());
    assert_eq!(again.flush_multiplier(), Some(0.5));
    // A printer's profile that gives a multiplier of its own does not
    // change the project's (the reference's project_config).
    let printer = edge_config::Settings::from_json(r#"{"flush_multiplier": "1.5"}"#).unwrap();
    let chosen = Chosen {
        machine: Some(("printer".into(), printer)),
        ..Chosen::default()
    };
    let job = edge_engine::prepare(
        again.place(Some(1)).unwrap(),
        &chosen,
        &Overrides::default(),
    )
    .unwrap();
    assert_eq!(job.settings.text("flush_multiplier"), Some("0.5"));
}
