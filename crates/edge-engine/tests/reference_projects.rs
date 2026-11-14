//! The engine as a caller sees it, on projects and profiles from the
//! reference.

use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use edge_engine::{Cancelled, Chosen, Library, Overrides, Stage, Unwatched, Watcher};

fn reference(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

fn job(project: &str) -> edge_engine::Job {
    let opened = edge_engine::open(&reference(project), None).unwrap();
    let loaded = opened.place().unwrap();
    edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap()
}

/// Notes each stage and stops the work once `stop_after` have started.
struct Follow {
    seen: Mutex<Vec<Stage>>,
    stop_after: usize,
    stopped: AtomicBool,
}

impl Watcher for Follow {
    fn stage(&self, stage: Stage) {
        let mut seen = self.seen.lock().unwrap();
        seen.push(stage);
        if seen.len() >= self.stop_after {
            self.stopped.store(true, Ordering::Relaxed);
        }
    }

    fn cancelled(&self) -> bool {
        self.stopped.load(Ordering::Relaxed)
    }
}

#[test]
fn a_project_is_sliced_drawn_and_written() {
    let sliced = edge_engine::slice(
        job("tests/data/support_corpus/twopart_groups.3mf"),
        &Unwatched,
    )
    .unwrap();
    assert_eq!(sliced.layers.len(), 60);
    // Every line of every layer is there to draw, once.
    for index in 0..sliced.layers.len() {
        let strokes = sliced.strokes(index);
        let lines: usize = (0..sliced.used.len())
            .map(|filament| {
                sliced.walls_by[filament][index].loops.len()
                    + sliced.fills_by[filament][index].len()
            })
            .sum::<usize>()
            + sliced.supports[index].len()
            + sliced.around[index].len();
        assert_eq!(strokes.len(), lines, "layer {index}");
        assert!(
            strokes
                .iter()
                .all(|stroke| stroke.width > 0.0 && stroke.points.len() >= 2)
        );
    }
    assert!(sliced.strokes(sliced.layers.len()).is_empty());
    let gcode = sliced.gcode("2026-11-10 at 00:00:00").unwrap();
    let text = String::from_utf8(gcode.text).unwrap();
    assert!(text.contains("on 2026-11-10 at 00:00:00"));
    assert!(gcode.summary.filament_mm > 500.0);
}

#[test]
fn a_tool_change_layer_draws_each_filament_and_the_tower() {
    let opened = edge_engine::open(&reference("tests/mcbrim_test.3mf"), Some(2)).unwrap();
    assert_eq!(opened.plate(), Some((2, 3)));
    assert_eq!(opened.plates(), [1, 2, 3]);
    let job = edge_engine::prepare(
        opened.place().unwrap(),
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let layer = (0..sliced.layers.len())
        .find(|index| sliced.changes[*index] > 1)
        .expect("a layer with tool changes");
    let strokes = sliced.strokes(layer);
    let mut filaments: Vec<usize> = strokes.iter().map(|stroke| stroke.filament).collect();
    filaments.dedup();
    assert!(filaments.len() > 1, "{filaments:?}");
    assert!(
        strokes
            .iter()
            .any(|stroke| stroke.role == edge_extrusion::Role::PrimeTower)
    );
}

#[test]
fn a_watcher_follows_the_stages_and_can_stop_them() {
    let all = Follow {
        seen: Mutex::new(Vec::new()),
        stop_after: usize::MAX,
        stopped: AtomicBool::new(false),
    };
    edge_engine::slice(job("tests/data/support_corpus/twopart_groups.3mf"), &all).unwrap();
    assert_eq!(*all.seen.lock().unwrap(), Stage::ALL);

    let early = Follow {
        seen: Mutex::new(Vec::new()),
        stop_after: 2,
        stopped: AtomicBool::new(false),
    };
    let stopped = edge_engine::slice(job("tests/data/support_corpus/twopart_groups.3mf"), &early)
        .err()
        .expect("stopped");
    assert!(stopped.is::<Cancelled>());
    assert_eq!(*early.seen.lock().unwrap(), [Stage::Layers, Stage::Walls]);
}

#[test]
fn a_vendors_presets_are_listed_by_printer() {
    let library = Library::new(reference("resources/profiles"));
    let vendors = library.vendors().unwrap();
    assert!(vendors.iter().any(|vendor| vendor == "Snapmaker"));
    let shelf = library.shelf("Snapmaker").unwrap();
    let machine = "Snapmaker U1 (0.4 nozzle)";
    assert!(shelf.machines().iter().any(|name| name == machine));
    // Presets that exist only to be inherited from are not offered.
    assert!(!shelf.machines().iter().any(|name| name.starts_with("fdm_")));
    let processes = shelf.processes_for(machine);
    let process = "0.20mm Standard @Snapmaker U1 (0.4 nozzle)";
    assert!(processes.contains(&process), "{processes:?}");
    let filaments = shelf.filaments_for(machine);
    assert!(!filaments.is_empty());
    // The printer's profile says which it usually prints with.
    let (usual_process, usual_filament) = shelf.usual_for(machine);
    assert_eq!(usual_process.as_deref(), Some(process));
    assert_eq!(
        usual_filament.as_deref(),
        Some("Generic PLA @U1 0.4 nozzle")
    );
    assert_eq!(shelf.usual_for("no such printer"), (None, None));
    // None of them is for another printer.
    assert!(shelf.processes_for("no such printer").is_empty());

    let chosen = shelf
        .choose(Some(machine), Some(process), &filaments[..1])
        .unwrap();
    assert_eq!(chosen.machine.unwrap().0, machine);
    assert!(chosen.process.unwrap().1.get("layer_height").is_some());
    assert!(shelf.choose(Some("no such printer"), None, &[]).is_err());
    assert!(shelf.choose(None, None, &["no such filament"]).is_err());
}

#[test]
fn a_filament_preset_serves_every_filament_of_the_model() {
    let library = Library::new(reference("resources/profiles"));
    let shelf = library.shelf("Snapmaker").unwrap();
    let machine = "Snapmaker U1 (0.4 nozzle)";
    let filament = shelf.filaments_for(machine)[0].to_owned();
    let two_filaments = || {
        let opened = edge_engine::open(
            &reference("tests/data/support_corpus/twopart_groups.3mf"),
            None,
        )
        .unwrap();
        let mut loaded = opened.place().unwrap();
        // The project describes two filaments; print part of it in a third.
        loaded.bodies[0].filament = 3;
        loaded
    };

    // With the project's own settings there is nothing to say of filament 3.
    let job = edge_engine::prepare(two_filaments(), &Chosen::default(), &Overrides::default());
    let sliced = edge_engine::slice(job.unwrap(), &Unwatched).unwrap();
    let refused = sliced.gcode("now").err().expect("no G-code").to_string();
    assert!(
        refused.contains("prints in filament 3") && refused.contains("only 2 filament"),
        "{refused}"
    );

    // A filament preset chosen is every filament's, and that is said.
    let chosen = shelf.choose(Some(machine), None, &[&filament]).unwrap();
    let job = edge_engine::prepare(two_filaments(), &chosen, &Overrides::default()).unwrap();
    assert_eq!(job.remarks.len(), 1);
    assert!(job.remarks[0].contains("3 filaments"), "{:?}", job.remarks);
    assert_eq!(job.settings.texts("nozzle_temperature").unwrap().len(), 3);
    assert_eq!(
        job.settings.texts("filament_settings_id").unwrap(),
        [filament.clone(), filament.clone(), filament.clone()]
    );
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    sliced.gcode("now").unwrap();

    // One filament in, one filament out: nothing is repeated or remarked on.
    let plain = edge_engine::open(
        &reference("tests/data/support_corpus/twopart_groups.3mf"),
        None,
    )
    .unwrap();
    let job = edge_engine::prepare(plain.place().unwrap(), &chosen, &Overrides::default()).unwrap();
    assert!(job.remarks.is_empty());
    assert_eq!(job.settings.texts("nozzle_temperature").unwrap().len(), 1);
}

/// Where the U1 of `mcbrim_test.3mf` (its filaments not rammed) puts each
/// tool away: with "Tool change on wipe tower" set, over the prime tower,
/// so that it waits there and not over the part while the next heats; as
/// the project has it, unset, where the nozzle is, except first thing on
/// a layer (GCode.cpp `append_tcr2`, `should_travel_to_tower`).
#[test]
fn a_tool_is_changed_over_the_prime_tower_where_that_is_set() {
    let put_away = |set: &str| {
        let opened = edge_engine::open(&reference("tests/mcbrim_test.3mf"), Some(2)).unwrap();
        let overrides = Overrides {
            settings: edge_config::Settings::from_json(set).unwrap(),
            ..Overrides::default()
        };
        let job =
            edge_engine::prepare(opened.place().unwrap(), &Chosen::default(), &overrides).unwrap();
        let sliced = edge_engine::slice(job, &Unwatched).unwrap();
        let [min_x, min_y, max_x, max_y] = sliced.tower.footprint.expect("a tower");
        let text = String::from_utf8(sliced.gcode("now").unwrap().text).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        let word = |line: &str, letter: char| -> Option<f64> {
            line.split_whitespace()
                .find_map(|word| word.strip_prefix(letter)?.parse().ok())
        };
        // For each change, whether the nozzle was last over the tower.
        lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.ends_with(";cooldown"))
            .map(|(at, _)| {
                let (x, y) = lines[..at]
                    .iter()
                    .rev()
                    .filter(|line| ["G1 ", "G2 ", "G3 "].iter().any(|g| line.starts_with(g)))
                    .find_map(|line| Some((word(line, 'X')?, word(line, 'Y')?)))
                    .expect("a move before the change");
                (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y)
            })
            .collect::<Vec<bool>>()
    };
    let set = put_away(r#"{"tool_change_on_wipe_tower": "1"}"#);
    assert!(set.len() > 20, "{} changes", set.len());
    assert!(
        set.iter().all(|over| *over),
        "a tool put away off the tower"
    );
    let unset = put_away("{}");
    assert_eq!(unset.len(), set.len());
    assert!(
        unset.iter().filter(|over| !**over).count() > set.len() / 2,
        "most tools put away where they are"
    );
}

#[test]
fn each_filament_can_have_a_preset_of_its_own() {
    let library = Library::new(reference("resources/profiles"));
    let shelf = library.shelf("Snapmaker").unwrap();
    let machine = "Snapmaker U1 (0.4 nozzle)";
    // Two filament presets that print at different temperatures.
    let temperature = |name: &str| {
        let chosen = shelf.choose(None, None, &[name]).unwrap();
        chosen.filaments[0].1.texts("nozzle_temperature").unwrap()[0].clone()
    };
    let names = shelf.filaments_for(machine);
    let first = names[0];
    let other = names
        .iter()
        .copied()
        .find(|name| temperature(name) != temperature(first))
        .expect("a filament that prints at another temperature");
    let model = |filament: usize| {
        let opened = edge_engine::open(
            &reference("tests/data/support_corpus/twopart_groups.3mf"),
            None,
        )
        .unwrap();
        assert_eq!(opened.filaments(), [1]);
        let mut loaded = opened.place().unwrap();
        loaded.bodies[0].filament = filament;
        loaded
    };

    // Filament 2 prints with the second preset.
    let chosen = shelf.choose(Some(machine), None, &[first, other]).unwrap();
    let job = edge_engine::prepare(model(2), &chosen, &Overrides::default()).unwrap();
    assert!(job.remarks.is_empty(), "{:?}", job.remarks);
    assert_eq!(
        job.settings.texts("nozzle_temperature").unwrap(),
        [temperature(first), temperature(other)]
    );
    assert_eq!(
        job.settings.texts("filament_settings_id").unwrap(),
        [first.to_owned(), other.to_owned()]
    );
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    sliced.gcode("now").unwrap();

    // A third filament with two presets named takes the second, and that is said.
    let job = edge_engine::prepare(model(3), &chosen, &Overrides::default()).unwrap();
    assert_eq!(
        job.settings.texts("nozzle_temperature").unwrap(),
        [temperature(first), temperature(other), temperature(other)]
    );
    assert!(
        job.remarks[0].contains("is used for the rest"),
        "{:?}",
        job.remarks
    );
}

#[test]
fn a_projects_plates_are_listed_with_what_is_on_each() {
    let project = reference("tests/mcbrim_test.3mf");
    let opened = edge_engine::open(&project, None).unwrap();
    let plates = opened.plate_list();
    assert_eq!(
        plates.iter().map(|plate| plate.id).collect::<Vec<_>>(),
        opened.plates()
    );
    assert_eq!(plates.len(), 3);
    for plate in &plates {
        // What a plate is listed with is what taking that plate gives.
        let taken = edge_engine::open(&project, Some(plate.id)).unwrap();
        assert_eq!(plate.filaments, taken.filaments(), "plate {}", plate.id);
        assert_eq!(
            plate.objects.is_empty(),
            plate.filaments.is_empty(),
            "plate {}",
            plate.id
        );
    }
    assert!(plates.iter().any(|plate| plate.filaments.len() > 1));
    // A bare model has no plates to list.
    let model = edge_engine::open(
        &reference("resources/handy_models/OrcaToleranceTest.stl"),
        None,
    )
    .unwrap();
    assert!(model.plate_list().is_empty());
}

/// A vendor's printer models, as the printer selection lists them: the
/// nozzles each is sold with, its picture, the presets made for it and
/// the sizes of its usual one.
/// A printer model's id, as the reference finds it for a sliced file's
/// `printer_model_id` (Preset.cpp `get_printer_type`): any vendor's model
/// of that name.
#[test]
fn a_printer_models_id_is_found_by_its_name() {
    let library = Library::new(reference("resources/profiles"));
    assert_eq!(library.model_id("Bambu Lab A1 mini").as_deref(), Some("N1"));
    assert_eq!(
        library.model_id("Bambu Lab X1 Carbon").as_deref(),
        Some("BL-P001")
    );
    assert_eq!(library.model_id("Snapmaker U1").as_deref(), Some("SM_U1"));
    assert_eq!(library.model_id("No Such Printer"), None);
    assert_eq!(library.model_id(""), None);
}

#[test]
fn a_vendors_printer_models_are_listed_with_their_presets() {
    let library = Library::new(reference("resources/profiles"));
    let models = library.models("Snapmaker").unwrap();
    // The index's own order: the U1 is its first entry.
    let u1 = &models[0];
    assert_eq!(u1.model, "Snapmaker U1");
    assert_eq!(u1.name, "Snapmaker U1");
    assert_eq!(u1.vendor, "Snapmaker");
    assert_eq!(u1.model_id.as_deref(), Some("SM_U1"));
    assert_eq!(u1.variants, ["0.2", "0.4", "0.6", "0.8", "0.4+0.6"]);
    assert_eq!(u1.bed_model.as_deref(), Some("Snapmaker U1_bed.stl"));
    assert_eq!(u1.bed_texture.as_deref(), Some("Snapmaker U1_texture.svg"));
    assert!(
        u1.default_materials
            .contains(&"Generic PLA @U1 0.4 nozzle".to_owned())
    );
    assert_eq!(u1.default_bed_type, None);
    let a1 = library
        .models("BBL")
        .unwrap()
        .into_iter()
        .find(|model| model.model == "Bambu Lab A1 mini")
        .unwrap();
    assert_eq!(a1.default_bed_type.as_deref(), Some("Textured PEI Plate"));
    let cover = u1.cover.as_ref().expect("the U1 has a picture");
    assert!(cover.is_file());
    assert_eq!(
        cover.file_name().and_then(|name| name.to_str()),
        Some("Snapmaker U1_cover.png")
    );
    // A preset for each nozzle, each naming the model and its nozzle.
    let preset = |variant: &str| {
        u1.machines
            .iter()
            .find(|preset| preset.variant == variant)
            .map(|preset| preset.name.as_str())
    };
    assert_eq!(preset("0.4"), Some("Snapmaker U1 (0.4 nozzle)"));
    assert_eq!(preset("0.2"), Some("Snapmaker U1 (0.2 nozzle)"));
    for variant in &u1.variants {
        assert!(
            preset(variant).is_some(),
            "no preset for a {variant} nozzle"
        );
    }
    // The sizes are those of the preset for a 0.4 nozzle: four toolheads.
    let bed = u1.bed.expect("the U1's bed is given");
    assert!(
        (bed[0] - 270.0).abs() < 2.0 && (bed[1] - 270.0).abs() < 2.0,
        "{bed:?}"
    );
    assert!(u1.height.is_some_and(|height| (height - 270.0).abs() < 2.0));
    assert_eq!(u1.nozzles, Some(4));

    // Every model of every vendor is listed, each preset under one model;
    // a model with a preset has its sizes read from one of them.
    let mut count = 0;
    for vendor in library.vendors().unwrap() {
        for model in library.models(&vendor).unwrap() {
            count += 1;
            assert_eq!(model.vendor, vendor);
            assert!(!model.name.is_empty());
            if !model.machines.is_empty() {
                assert!(model.nozzles.is_some(), "{vendor} {}", model.model);
            }
        }
    }
    assert!(count > 300, "only {count} models were found");
}

/// A vendor that is not there is an error; one without printers lists
/// none; a model whose file is missing is left out and the rest stay.
#[test]
fn printer_models_of_a_folder_made_for_the_test() {
    let dir = tempfile::tempdir().unwrap();
    let library = Library::new(dir.path());
    assert!(library.models("Nobody").is_err());

    std::fs::write(dir.path().join("Empty.json"), r#"{ "name": "Empty" }"#).unwrap();
    assert_eq!(library.models("Empty").unwrap(), []);

    let vendor = dir.path().join("Maker");
    std::fs::create_dir_all(vendor.join("machine")).unwrap();
    std::fs::write(
        dir.path().join("Maker.json"),
        r#"{
            "name": "Maker",
            "machine_model_list": [
                { "name": "Maker One", "sub_path": "machine/Maker One.json" },
                { "name": "Maker Gone", "sub_path": "machine/Maker Gone.json" }
            ],
            "machine_list": [
                { "name": "common", "sub_path": "machine/common.json" },
                { "name": "Maker One 0.6 nozzle", "sub_path": "machine/one06.json" },
                { "name": "Maker One HF 0.4 nozzle", "sub_path": "machine/one04hf.json" },
                { "name": "Maker One 0.4 nozzle", "sub_path": "machine/one04.json" }
            ]
        }"#,
    )
    .unwrap();
    let write =
        |name: &str, text: &str| std::fs::write(vendor.join("machine").join(name), text).unwrap();
    write(
        "Maker One.json",
        r#"{ "type": "machine_model", "name": "Maker One", "nozzle_diameter": "0.6; 0.4", "default_materials": "PLA A;PLA B;" }"#,
    );
    write(
        "common.json",
        r#"{ "type": "machine", "name": "common", "instantiation": "false", "printer_model": "Maker One",
             "printable_area": ["0x0", "200x0", "200x180", "0x180"], "printable_height": "150" }"#,
    );
    write(
        "one06.json",
        r#"{ "type": "machine", "name": "Maker One 0.6 nozzle", "inherits": "common", "instantiation": "true",
             "printer_variant": "0.6", "nozzle_diameter": ["0.6"], "printable_height": "140" }"#,
    );
    write(
        "one04hf.json",
        r#"{ "type": "machine", "name": "Maker One HF 0.4 nozzle", "inherits": "common", "instantiation": "true",
             "printer_variant": "0.4", "nozzle_diameter": ["0.4", "0.4"] }"#,
    );
    write(
        "one04.json",
        r#"{ "type": "machine", "name": "Maker One 0.4 nozzle", "inherits": "common", "instantiation": "true",
             "printer_variant": "0.4", "nozzle_diameter": ["0.4"] }"#,
    );
    let models = library.models("Maker").unwrap();
    assert_eq!(models.len(), 1, "the model without a file is left out");
    let one = &models[0];
    assert_eq!(one.variants, ["0.6", "0.4"]);
    assert_eq!(one.default_materials, ["PLA A", "PLA B"]);
    assert_eq!(one.cover, None);
    assert_eq!(one.model_id, None);
    // The presets that can be picked, in the index's order; the one that
    // is only inherited from is not among them.
    let names: Vec<&str> = one
        .machines
        .iter()
        .map(|preset| preset.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Maker One 0.6 nozzle",
            "Maker One HF 0.4 nozzle",
            "Maker One 0.4 nozzle"
        ]
    );
    // Sizes from the 0.4 preset with the shortest name, not the first.
    assert_eq!(one.bed, Some([200.0, 180.0]));
    assert_eq!(one.height, Some(150.0));
    assert_eq!(one.nozzles, Some(1));
}

/// What a filament preset is made of is known to its vendor's shelf.
#[test]
fn a_shelf_knows_what_a_filament_is_made_of() {
    let library = Library::new(reference("resources/profiles"));
    let shelf = library.shelf("Snapmaker").unwrap();
    assert_eq!(shelf.filament_type("Snapmaker PLA @U1"), Some("PLA"));
    assert_eq!(shelf.filament_type("no such filament"), None);
}

#[test]
fn a_project_is_a_scene_of_objects_on_their_plates() {
    use edge_engine::Mark;
    let opened = edge_engine::open(&reference("tests/mcbrim_test.3mf"), None).unwrap();
    let scene = opened.scene();
    let plates: Vec<u32> = scene.plates.iter().map(|plate| plate.id).collect();
    assert_eq!(plates, [1, 2, 3]);
    assert!(!scene.bare && !scene.objects.is_empty());
    // Every copy stands on a plate of the file, and what it holds is what
    // slicing takes from that plate: the same triangles.
    for (index, plate) in opened.plate_list().iter().enumerate() {
        let on_it: Vec<_> = scene
            .placings
            .iter()
            .filter(|placing| placing.plate == Some(index) && placing.printable)
            .collect();
        assert_eq!(on_it.len(), plate.objects.len(), "plate {}", plate.id);
        let printed: usize = on_it
            .iter()
            .map(|placing| {
                scene.objects[placing.object]
                    .marks
                    .iter()
                    .filter(|mark| matches!(mark, Mark::Filament(_)))
                    .count()
            })
            .sum();
        let loaded = edge_engine::open(&reference("tests/mcbrim_test.3mf"), Some(plate.id))
            .unwrap()
            .place()
            .unwrap();
        let sliced: usize = loaded.objects.iter().map(|mesh| mesh.triangles.len()).sum();
        // Paint cuts triangles up, so a painted object has more to show.
        assert!(printed >= sliced && sliced > 0, "plate {}", plate.id);
        // Measured from its own plate: where slicing has it.
        let (low, high) = loaded.bounds;
        for placing in on_it {
            let object = &scene.objects[placing.object];
            let (min, max) = object.bounds.unwrap();
            let centre = placing.transform.apply(std::array::from_fn(|axis| {
                f64::from(min[axis] + max[axis]) / 2.0
            }));
            for axis in 0..2 {
                assert!(
                    centre[axis] >= f64::from(low[axis]) - 1e-3
                        && centre[axis] <= f64::from(high[axis]) + 1e-3,
                    "plate {}: {centre:?} outside {low:?}..{high:?}",
                    plate.id
                );
            }
        }
    }
    for object in &scene.objects {
        assert_eq!(object.marks.len(), object.triangles.len());
        let count = object.vertices.len() as u32;
        assert!(object.triangles.iter().flatten().all(|at| *at < count));
    }
    // A bare model is one object on no plate.
    let bare = edge_engine::open(
        &reference("resources/handy_models/OrcaToleranceTest.stl"),
        None,
    )
    .unwrap()
    .scene();
    assert!(bare.bare && bare.plates.is_empty());
    assert_eq!((bare.objects.len(), bare.placings.len()), (1, 1));
}

#[test]
fn the_gcode_reads_back_as_the_moves_of_the_print() {
    use edge_engine::{Feature, MoveKind};
    let sliced = edge_engine::slice(
        job("tests/data/support_corpus/twopart_groups.3mf"),
        &Unwatched,
    )
    .unwrap();
    let gcode = sliced.gcode("2026-11-10 at 00:00:00").unwrap();
    let path = edge_engine::read_toolpath(&gcode.text);
    // A layer for each layer sliced, each at its height.
    assert_eq!(path.layers.len(), sliced.layers.len());
    for (index, layer) in path.layers.iter().enumerate() {
        assert!((f64::from(layer.z) - sliced.top_of(index)).abs() < 1e-3);
        assert!((f64::from(layer.height) - sliced.height_of(index)).abs() < 1e-3);
        assert_eq!(path.moves[layer.first].layer as usize, index);
    }
    assert_eq!(path.layers[0].first, 0);
    assert!(
        path.moves
            .windows(2)
            .all(|pair| pair[0].layer <= pair[1].layer)
    );

    // The filament laid is the filament the writer counted, bar what the
    // start G-code purges, which the reader shows as the file's own.
    let mut laid = 0.0;
    let mut features = std::collections::BTreeSet::new();
    for pair in path.moves.windows(2) {
        let (from, to) = (pair[0], pair[1]);
        if to.kind != MoveKind::Extrude {
            continue;
        }
        features.insert(to.feature);
        assert!(to.width > 0.05 && to.height > 0.05, "{to:?}");
        if to.feature != Feature::Custom {
            let length = (0..2)
                .map(|axis| f64::from(to.to[axis] - from.to[axis]).powi(2))
                .sum::<f64>()
                .sqrt();
            // The reference's cross-section of a line: a rectangle with
            // rounded ends.
            let (w, h) = (f64::from(to.width), f64::from(to.height));
            laid += length * (h * (w - h) + std::f64::consts::FRAC_PI_4 * h * h);
        }
    }
    let counted = gcode.summary.filament_mm3;
    assert!(
        (laid - counted).abs() < 0.05 * counted,
        "{laid} mm3 drawn, {counted} mm3 fed"
    );
    assert!(features.contains(&Feature::OuterWall) && features.contains(&Feature::Custom));
    // What moves between lines is there too, and a seam for an outer
    // wall that closes.
    let count = |kind| path.moves.iter().filter(|step| step.kind == kind).count();
    // The last retraction of a print is not taken back.
    assert!(count(MoveKind::Retract).abs_diff(count(MoveKind::Unretract)) <= 1);
    assert!(count(MoveKind::Retract) >= gcode.summary.retractions);
    assert!(count(MoveKind::Travel) > 60 && count(MoveKind::Wipe) > 60);
    // (One that starts as an overhang wall is not told to be closed, by
    // the reference either.)
    let seams = count(MoveKind::Seam);
    assert!(
        seams + 3 >= sliced.layers.len() && seams <= 2 * sliced.layers.len(),
        "{seams} seams"
    );
    // The same file reads the same.
    assert_eq!(path, edge_engine::read_toolpath(&gcode.text));

    // Without G-code, the lines alone.
    let lines = sliced.toolpath_of_lines();
    assert_eq!(lines.layers.len(), sliced.layers.len());
    let strokes: usize = (0..sliced.layers.len())
        .map(|index| sliced.strokes(index).len())
        .sum();
    assert_eq!(
        lines
            .moves
            .iter()
            .filter(|step| step.kind == MoveKind::Start)
            .count(),
        strokes
    );
    assert!(
        lines
            .moves
            .iter()
            .all(|step| matches!(step.kind, MoveKind::Start | MoveKind::Extrude))
    );
}
