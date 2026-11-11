//! Ironing through the whole pipeline, on a part with a step in it:
//! which layers are ironed for each `ironing_type`, that the G-code says
//! so, and that it comes out the same however many threads make it.

use std::sync::atomic::{AtomicUsize, Ordering};

use edge_config::Value;
use edge_engine::{Chosen, Overrides, Sliced, Unwatched};
use edge_extrusion::Role;

/// A part with a step in it: a 20 mm square base 2 mm tall carrying a
/// 10 mm square block to 4 mm, sliced with the reference's defaults and
/// `changes`.
fn sliced(changes: &[(&str, &str)]) -> Sliced {
    let mut soup = edge_mesh::primitives::cuboid([0.0, 0.0, 0.0], [20.0, 20.0, 2.0]);
    soup.extend(edge_mesh::primitives::cuboid(
        [0.0, 0.0, 2.0],
        [10.0, 10.0, 4.0],
    ));
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    // A file of its own for each test, which run side by side.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-ironing-{}-{}.stl",
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

/// The layers with ironing on them, and how much of it, in mm.
fn ironed(sliced: &Sliced) -> Vec<(usize, f64)> {
    (0..sliced.layers.len())
        .filter_map(|index| {
            let length: f64 = sliced
                .fills_by
                .iter()
                .flat_map(|fills| &fills[index])
                .filter(|line| line.role == Role::Ironing)
                .map(edge_extrusion::ExtrusionPath::length_mm)
                .sum();
            (length > 0.0).then_some((index, length))
        })
        .collect()
}

fn gcode(sliced: &Sliced) -> Vec<u8> {
    sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text
}

#[test]
fn nothing_is_ironed_unless_asked_and_the_file_is_what_it_was() {
    let plain = sliced(&[]);
    assert!(ironed(&plain).is_empty());
    let off = sliced(&[("ironing_type", "no ironing"), ("ironing_spacing", "0.3")]);
    assert!(ironed(&off).is_empty());
    // Every line of fill is what it is without the setting; the file
    // differs only where it lists its settings.
    assert_eq!(plain.fills_by, off.fills_by);
    let text = String::from_utf8(gcode(&plain)).unwrap();
    assert!(!text.contains(";TYPE:Ironing"));
}

#[test]
fn each_kind_of_ironing_takes_the_layers_it_names() {
    let top = sliced(&[("ironing_type", "top")]);
    let topmost = sliced(&[("ironing_type", "topmost")]);
    let solid = sliced(&[("ironing_type", "solid")]);
    let last = top.layers.len() - 1;
    let layers = |sliced: &Sliced| -> Vec<usize> {
        ironed(sliced).into_iter().map(|(index, _)| index).collect()
    };

    // Layers 0.2 mm thick: the ledge beside the block is the tenth
    // layer's top surface and the block's top the twentieth's.
    assert_eq!(last, 19);
    let tops = ironed(&top);
    assert_eq!(layers(&top), [9, last]);
    // The ledge is ironed right up to the block: 400 - 100 mm² less
    // half a nozzle round the outside, in lines 0.1 mm apart, and the
    // block's top the same way.
    let expect =
        |found: f64, area: f64| assert!((found - area / 0.1).abs() < 0.02 * found, "{found}");
    expect(tops[0].1, 19.6 * 19.6 - 9.8 * 9.8);
    expect(tops[1].1, 9.6 * 9.6);
    // Only the last layer is the topmost.
    assert_eq!(layers(&topmost), [last]);
    assert!((ironed(&topmost)[0].1 - tops[1].1).abs() < 1e-9);
    // Every solid layer: all that `top` irons, the layers on the bed
    // and under each top surface besides, each of those whole, and
    // nothing of a layer that is sparse right through.
    let solids = ironed(&solid);
    let on = |index: usize| {
        solids
            .iter()
            .find(|(layer, _)| *layer == index)
            .map(|found| found.1)
    };
    assert!(on(9).is_some() && on(last).is_some());
    expect(on(0).unwrap(), 19.6 * 19.6);
    expect(on(last - 1).unwrap(), 9.6 * 9.6);
    assert!(on(last - 1).unwrap() > 0.0 && on(12).is_none());
    assert!(solids.len() > 2 && solids.len() < top.layers.len());

    // Ironing lines are thin: a tenth of a layer shared out over lines
    // a quarter of a nozzle apart, and written under their own name.
    let line = top
        .fills_by
        .iter()
        .flatten()
        .flatten()
        .find(|line| line.role == Role::Ironing)
        .unwrap();
    assert!(line.flow.height < 0.02 && line.flow.height > 0.0);
    let text = String::from_utf8(gcode(&top)).unwrap();
    assert_eq!(
        text.matches(";TYPE:Ironing").count(),
        tops.len(),
        "one stretch of ironing on each ironed layer of this part"
    );
    // And last on its layer: nothing else is laid after it there.
    let ironed_layer = text.rsplit(";LAYER_CHANGE").next().unwrap();
    let after = ironed_layer.split(";TYPE:Ironing").nth(1).unwrap();
    assert!(!after.contains(";TYPE:Top surface") && !after.contains(";TYPE:Outer wall"));
    // Nothing is said of ironing among what is not followed.
    assert!(!top.notes().any(|note| note.contains("ironing")));
}

#[test]
fn ironing_comes_out_the_same_on_one_thread_and_on_many() {
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| gcode(&sliced(&[("ironing_type", "solid")])))
    };
    let one = run(1);
    assert!(one == run(4) && one == run(1));
}
