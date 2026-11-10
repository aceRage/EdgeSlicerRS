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
    // None of them is for another printer.
    assert!(shelf.processes_for("no such printer").is_empty());

    let chosen = shelf
        .choose(Some(machine), Some(process), Some(filaments[0]))
        .unwrap();
    assert_eq!(chosen.machine.unwrap().0, machine);
    assert!(chosen.process.unwrap().1.get("layer_height").is_some());
    assert!(shelf.choose(Some("no such printer"), None, None).is_err());
}
