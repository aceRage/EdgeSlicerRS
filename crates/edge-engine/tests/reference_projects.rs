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

#[test]
fn a_tool_is_changed_over_the_prime_tower() {
    let opened = edge_engine::open(&reference("tests/mcbrim_test.3mf"), Some(2)).unwrap();
    let job = edge_engine::prepare(
        opened.place().unwrap(),
        &Chosen::default(),
        &Overrides::default(),
    )
    .unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let [min_x, min_y, max_x, max_y] = sliced.tower.footprint.expect("a tower");
    let text = String::from_utf8(sliced.gcode("now").unwrap().text).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let word = |line: &str, letter: char| -> Option<f64> {
        line.split_whitespace()
            .find_map(|word| word.strip_prefix(letter)?.parse().ok())
    };
    let mut changes = 0;
    for (at, _) in lines
        .iter()
        .enumerate()
        .filter(|(_, line)| line.ends_with(";cooldown"))
    {
        // Wherever the nozzle last went before it was put away.
        let (x, y) = lines[..at]
            .iter()
            .rev()
            .filter(|line| line.starts_with("G1 "))
            .find_map(|line| Some((word(line, 'X')?, word(line, 'Y')?)))
            .expect("a move before the change");
        assert!(
            (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y),
            "tool put away at {x} x {y}, off the tower"
        );
        changes += 1;
    }
    assert!(changes > 20, "{changes} changes");
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
