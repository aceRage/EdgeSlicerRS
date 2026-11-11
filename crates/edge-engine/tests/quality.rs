//! Two of the Quality tab's options through the whole pipeline: a
//! precise height (`precise_z_height`) and overhangs made printable
//! (`make_overhang_printable`). That each changes what it should and
//! nothing else, that the G-code follows, and that the result is the
//! same however many threads make it.

use std::sync::atomic::{AtomicUsize, Ordering};

use edge_config::Value;
use edge_engine::{Chosen, Overrides, Sliced, Unwatched};
use edge_geom::ExPolygon;
use edge_mesh::primitives::cuboid;

/// Boxes as one part, sliced with the reference's defaults and `changes`.
fn sliced(boxes: &[([f32; 3], [f32; 3])], changes: &[(&str, &str)]) -> Sliced {
    let soup: Vec<_> = boxes
        .iter()
        .flat_map(|(min, max)| cuboid(*min, *max))
        .collect();
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    // A file of its own for each test, which run side by side.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-quality-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let opened = edge_engine::open(&path, None);
    std::fs::remove_file(&path).unwrap();
    let loaded = opened.unwrap().place().unwrap();
    let mut job = edge_engine::prepare(loaded, &Chosen::default(), &Overrides::default()).unwrap();
    for (key, value) in changes {
        job.settings.insert(*key, Value::One((*value).to_owned()));
    }
    edge_engine::slice(job, &Unwatched).unwrap()
}

/// A 20 mm square block 10.07 mm tall: at 0.2 mm a layer it ends 0.07 mm
/// short of a whole number of layers.
const BLOCK: [([f32; 3], [f32; 3]); 1] = [([0.0, 0.0, 0.0], [20.0, 20.0, 10.07])];

/// A 10 mm stem 10 mm tall under a 30 mm cap 4 mm thick.
const MUSHROOM: [([f32; 3], [f32; 3]); 2] = [
    ([10.0, 10.0, 0.0], [20.0, 20.0, 10.0]),
    ([0.0, 0.0, 10.0], [30.0, 30.0, 14.0]),
];

/// A 20 mm block 6 mm tall with a 6 mm square pocket 3 mm up into it
/// from underneath: four walls round the pocket and a lid on them.
const POCKET: [([f32; 3], [f32; 3]); 5] = [
    ([0.0, 0.0, 0.0], [20.0, 7.0, 3.0]),
    ([0.0, 13.0, 0.0], [20.0, 20.0, 3.0]),
    ([0.0, 7.0, 0.0], [7.0, 13.0, 3.0]),
    ([13.0, 7.0, 0.0], [20.0, 13.0, 3.0]),
    ([0.0, 0.0, 3.0], [20.0, 20.0, 6.0]),
];

fn gcode(sliced: &Sliced) -> String {
    String::from_utf8(sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text).unwrap()
}

fn area(regions: &[ExPolygon]) -> f64 {
    regions.iter().map(ExPolygon::area_mm2).sum()
}

fn noted(sliced: &Sliced, key: &str) -> Vec<String> {
    sliced
        .notes()
        .filter(|note| note.starts_with(key))
        .cloned()
        .collect()
}

#[test]
fn with_both_options_off_everything_is_as_it_was() {
    let plain = sliced(&MUSHROOM, &[]);
    let off = sliced(
        &MUSHROOM,
        &[
            ("precise_z_height", "0"),
            ("make_overhang_printable", "0"),
            ("make_overhang_printable_angle", "20"),
            ("make_overhang_printable_hole_size", "50"),
        ],
    );
    assert_eq!(plain.walls_by, off.walls_by);
    assert_eq!(plain.fills_by, off.fills_by);
    assert_eq!(plain.supports, off.supports);
    for index in 0..plain.layers.len() {
        assert_eq!(plain.layers[index].regions, off.layers[index].regions);
        assert_eq!(plain.layers[index].z, off.layers[index].z);
        assert_eq!(plain.height_of(index), off.height_of(index));
        assert_eq!(plain.top_of(index), off.top_of(index));
        // Every layer at the height set, its top worked out from its
        // number.
        let (first, rest) = (plain.first_layer_height, plain.layer_height);
        assert_eq!(
            plain.height_of(index),
            if index == 0 { first } else { rest }
        );
        assert_eq!(plain.top_of(index), first + index as f64 * rest);
    }
    assert!(noted(&off, "precise_z_height").is_empty());
    assert!(noted(&off, "make_overhang_printable").is_empty());
}

#[test]
fn a_precise_height_tunes_the_last_five_layers_and_no_others() {
    let plain = sliced(&BLOCK, &[]);
    let precise = sliced(&BLOCK, &[("precise_z_height", "1")]);
    let count = plain.layers.len();
    assert_eq!(precise.layers.len(), count);
    let height = f64::from(10.07_f32);
    let last = count - 1;
    // As set, the print ends where a whole layer does; tuned, at the
    // top of the part.
    assert!((plain.top_of(last) - height).abs() > 0.05);
    assert!(
        (precise.top_of(last) - height).abs() < 1e-9,
        "{}",
        precise.top_of(last)
    );
    let change = (height - plain.top_of(last)) / 5.0;
    for index in 0..count {
        let tuned = index + 5 >= count;
        if tuned {
            let wanted = plain.height_of(index) + change;
            assert!((precise.height_of(index) - wanted).abs() < 1e-9);
            // Cut through its middle, and laid at its own height.
            let middle = precise.top_of(index) - precise.height_of(index) / 2.0;
            assert!((precise.layers[index].z - middle).abs() < 1e-9);
            assert!((precise.top_of(index) - precise.top_of(index - 1) - wanted).abs() < 1e-9);
        } else {
            assert_eq!(precise.height_of(index), plain.height_of(index));
            assert_eq!(precise.top_of(index), plain.top_of(index));
            assert_eq!(precise.layers[index].z, plain.layers[index].z);
            assert_eq!(precise.walls_by[0][index], plain.walls_by[0][index]);
        }
        for wall in &precise.walls_by[0][index].loops {
            assert_eq!(wall.flow.height, precise.height_of(index));
        }
        // Fill is as thick as its layer too, but for what is laid as a
        // round thread over fill or over air.
        for line in &precise.fills_by[0][index] {
            if line.flow.height != line.flow.width {
                assert_eq!(line.flow.height, precise.height_of(index), "layer {index}");
            }
        }
    }
    assert!(noted(&precise, "precise_z_height").is_empty());

    // The file ends at the part's height, and says so.
    let text = gcode(&precise);
    assert!(text.contains("; max_z_height: 10.07"), "max_z_height");
    assert!(text.contains(";Z:10.070"));
    assert!(!gcode(&plain).contains(";Z:10.070"));
}

#[test]
fn a_part_too_short_to_tune_is_said_and_left_alone() {
    // 0.87 mm: four layers.
    let short = [([0.0, 0.0, 0.0], [20.0, 20.0, 0.87])];
    let plain = sliced(&short, &[]);
    let precise = sliced(&short, &[("precise_z_height", "1")]);
    assert!(plain.layers.len() < 6);
    assert_eq!(plain.walls_by, precise.walls_by);
    assert_eq!(plain.fills_by, precise.fills_by);
    let notes = noted(&precise, "precise_z_height");
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(notes[0].contains("six are needed"));
}

#[test]
fn an_overhang_made_printable_is_carried_down_at_the_angle_asked() {
    let plain = sliced(&MUSHROOM, &[]);
    let printable = sliced(&MUSHROOM, &[("make_overhang_printable", "1")]);
    let count = plain.layers.len();
    assert_eq!(printable.layers.len(), count);
    // The cap starts at the first layer cut above 10 mm.
    let cap = (0..count)
        .find(|index| area(&plain.layers[*index].regions) > 800.0)
        .unwrap();
    let reach = 55f64.to_radians().tan() * plain.layer_height;
    for index in 0..count {
        let (was, is) = (
            area(&plain.layers[index].regions),
            area(&printable.layers[index].regions),
        );
        if index >= cap {
            assert_eq!(printable.layers[index].regions, plain.layers[index].regions);
            continue;
        }
        // Each layer under the cap is in by `reach` on every side from
        // the one above, until the stem is all there is.
        let side = (30.0 - 2.0 * reach * (cap - index) as f64).max(10.0);
        assert!(
            (is - side * side).abs() < 1e-3,
            "layer {index}: {is} mm², wanted {}",
            side * side
        );
        assert!(is >= was);
    }
    // Where nothing was added the walls are the very ones.
    assert_eq!(printable.walls_by[0][0], plain.walls_by[0][0]);
    assert!(printable.walls_by[0][cap - 1] != plain.walls_by[0][cap - 1]);
    assert!(noted(&printable, "make_overhang_printable").is_empty());

    // 90 degrees allows any overhang: nothing changes.
    let upright = sliced(
        &MUSHROOM,
        &[
            ("make_overhang_printable", "1"),
            ("make_overhang_printable_angle", "90"),
        ],
    );
    assert_eq!(upright.walls_by, plain.walls_by);
    assert_eq!(upright.fills_by, plain.fills_by);
}

#[test]
fn a_pocket_underneath_is_filled_or_kept_by_its_size() {
    let plain = sliced(&POCKET, &[]);
    let holes = |sliced: &Sliced, index: usize| -> usize {
        sliced.layers[index]
            .regions
            .iter()
            .map(|region| region.holes.len())
            .sum()
    };
    assert_eq!(holes(&plain, 0), 1);
    // Filled to the bed by default.
    let filled = sliced(&POCKET, &[("make_overhang_printable", "1")]);
    for index in 0..filled.layers.len() {
        assert_eq!(holes(&filled, index), 0, "layer {index}");
    }
    // Kept when it is smaller than the size given: 36 mm² under 40.
    let kept = sliced(
        &POCKET,
        &[
            ("make_overhang_printable", "1"),
            ("make_overhang_printable_hole_size", "40"),
        ],
    );
    assert_eq!(kept.walls_by, plain.walls_by);
    assert_eq!(kept.fills_by, plain.fills_by);
    // And filled when it is larger.
    let larger = sliced(
        &POCKET,
        &[
            ("make_overhang_printable", "1"),
            ("make_overhang_printable_hole_size", "30"),
        ],
    );
    assert_eq!(larger.walls_by, filled.walls_by);
}

#[test]
fn both_options_come_out_the_same_on_one_thread_and_on_many() {
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            gcode(&sliced(
                &MUSHROOM,
                &[
                    ("precise_z_height", "1"),
                    ("make_overhang_printable", "1"),
                    ("make_overhang_printable_angle", "40"),
                ],
            ))
        })
    };
    let one = run(1);
    assert!(one == run(8), "the file differs with the number of threads");
    // The mushroom is 14 mm tall and its layers end there already: the
    // precise height has nothing to do, and the slope is in the file.
    assert!(one.contains("; max_z_height: 14.00"));
}
