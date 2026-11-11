//! Shells as thick as asked (`top_shell_thickness`,
//! `bottom_shell_thickness`) through the whole pipeline: that layers are
//! made solid until a shell measures what is set, that nothing changes
//! where the count of layers already gives it, that the layers are
//! measured as they lie, and that the result is the same however many
//! threads make it.

use std::sync::atomic::{AtomicUsize, Ordering};

use edge_config::Value;
use edge_engine::{Chosen, Overrides, Sliced, Unwatched};
use edge_infill::SurfaceKind;

/// A 20 mm square block `tall` mm high, sliced in layers of
/// `layer_height` mm with the reference's defaults and `changes`.
fn sliced(tall: f32, layer_height: f64, changes: &[(&str, &str)]) -> Sliced {
    let soup = edge_mesh::primitives::cuboid([0.0, 0.0, 0.0], [20.0, 20.0, tall]);
    let mesh = edge_mesh::Mesh::from_triangle_soup(soup);
    // A file of its own for each test, which run side by side.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "edge-shells-{}-{}.stl",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let opened = edge_engine::open(&path, None);
    std::fs::remove_file(&path).unwrap();
    let loaded = opened.unwrap().place().unwrap();
    let overrides = Overrides {
        layer_height: Some(layer_height),
        first_layer_height: Some(0.2),
        ..Overrides::default()
    };
    let mut job = edge_engine::prepare(loaded, &Chosen::default(), &overrides).unwrap();
    for (key, value) in changes {
        job.settings.insert(*key, Value::One((*value).to_owned()));
    }
    edge_engine::slice(job, &Unwatched).unwrap()
}

/// The layers that are solid inside and nothing else: no face, and no
/// sparse fill.
fn solid(sliced: &Sliced) -> Vec<usize> {
    let inside = |kind: SurfaceKind| {
        matches!(
            kind,
            SurfaceKind::InternalSolid | SurfaceKind::InternalBridge
        )
    };
    (0..sliced.layers.len())
        .filter(|layer| {
            let surfaces = &sliced.surfaces_by[0][*layer];
            !surfaces.is_empty() && surfaces.iter().all(|surface| inside(surface.kind))
        })
        .collect()
}

fn gcode(sliced: &Sliced) -> String {
    String::from_utf8(sliced.gcode("1970-01-01 at 00:00:01 UTC").unwrap().text).unwrap()
}

/// The file but for where it says what the two thicknesses were set to.
fn printed(sliced: &Sliced) -> String {
    gcode(sliced)
        .lines()
        .filter(|line| !line.contains("_shell_thickness"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn noted(sliced: &Sliced, key: &str) -> Vec<String> {
    sliced
        .notes()
        .filter(|note| note.starts_with(key))
        .cloned()
        .collect()
}

#[test]
fn a_top_shell_is_made_as_thick_as_asked() {
    // 8 mm in layers of 0.16 mm: fifty layers, the top on layer 49.
    let counted = sliced(
        8.0,
        0.16,
        &[
            ("top_shell_layers", "6"),
            ("top_shell_thickness", "0"),
            ("bottom_shell_layers", "3"),
        ],
    );
    assert_eq!(counted.layers.len(), 50);
    assert_eq!(solid(&counted), [1, 2, 44, 45, 46, 47, 48]);
    // Six layers are 0.96 mm, short of the 1 mm asked: one more.
    let asked = sliced(
        8.0,
        0.16,
        &[
            ("top_shell_layers", "6"),
            ("top_shell_thickness", "1.0"),
            ("bottom_shell_layers", "3"),
        ],
    );
    assert_eq!(solid(&asked), [1, 2, 43, 44, 45, 46, 47, 48]);
    assert!(noted(&asked, "top_shell_thickness").is_empty());
    // Below the shell nothing has moved.
    assert_eq!(asked.surfaces_by[0][..42], counted.surfaces_by[0][..42]);
    assert_eq!(asked.fills_by[0][..42], counted.fills_by[0][..42]);
    assert_eq!(asked.walls_by, counted.walls_by);
}

#[test]
fn a_bottom_shell_is_made_as_thick_as_asked() {
    let asked = sliced(
        8.0,
        0.16,
        &[
            ("top_shell_layers", "3"),
            ("top_shell_thickness", "0"),
            ("bottom_shell_layers", "3"),
            ("bottom_shell_thickness", "0.8"),
        ],
    );
    // A first layer of 0.2 mm and four of 0.16 mm are 0.84 mm; layer 5
    // starts above the 0.8 mm asked.
    assert_eq!(solid(&asked), [1, 2, 3, 4, 47, 48]);
    assert!(noted(&asked, "bottom_shell_thickness").is_empty());
}

#[test]
fn a_shell_the_count_makes_thick_enough_is_left_as_it_was() {
    // Five layers of 0.2 mm are the 1 mm asked.
    let layers = [("top_shell_layers", "5"), ("bottom_shell_layers", "3")];
    let with = |top: &'static str, bottom: &'static str| {
        let mut changes = layers.to_vec();
        changes.push(("top_shell_thickness", top));
        changes.push(("bottom_shell_thickness", bottom));
        sliced(8.0, 0.2, &changes)
    };
    let counted = with("0", "0");
    assert_eq!(solid(&counted), [1, 2, 35, 36, 37, 38]);
    for (top, bottom) in [("1.0", "0"), ("1", "0.6"), ("0.3", "0.2")] {
        let asked = with(top, bottom);
        assert_eq!(asked.surfaces_by, counted.surfaces_by, "{top} and {bottom}");
        assert_eq!(asked.fills_by, counted.fills_by, "{top} and {bottom}");
        assert!(
            printed(&asked) == printed(&counted),
            "the file differs with {top} and {bottom} mm asked"
        );
    }
}

#[test]
fn with_no_shell_layers_a_thickness_makes_no_shell() {
    let none = sliced(
        8.0,
        0.16,
        &[
            ("top_shell_layers", "0"),
            ("top_shell_thickness", "0"),
            ("bottom_shell_layers", "0"),
            ("bottom_shell_thickness", "0"),
        ],
    );
    let asked = sliced(
        8.0,
        0.16,
        &[
            ("top_shell_layers", "0"),
            ("top_shell_thickness", "1.0"),
            ("bottom_shell_layers", "0"),
            ("bottom_shell_thickness", "1.0"),
        ],
    );
    assert!(solid(&asked).is_empty());
    assert_eq!(asked.surfaces_by, none.surfaces_by);
    assert!(printed(&asked) == printed(&none));
}

#[test]
fn a_shell_is_measured_in_the_layers_a_precise_height_has_thinned() {
    // 7.95 mm at 0.2 mm: forty layers end at 8 mm, and with a precise
    // height the last five are 0.19 mm each.
    let changes = [
        ("top_shell_layers", "5"),
        ("top_shell_thickness", "1.0"),
        ("bottom_shell_layers", "3"),
    ];
    let plain = sliced(7.95, 0.2, &changes);
    assert_eq!(plain.layers.len(), 40);
    assert_eq!(solid(&plain), [1, 2, 35, 36, 37, 38]);
    let mut precise = changes.to_vec();
    precise.push(("precise_z_height", "1"));
    let precise = sliced(7.95, 0.2, &precise);
    assert!((precise.height_of(39) - 0.19).abs() < 1e-6);
    // Five of them are 0.95 mm: a sixth layer is made solid.
    assert_eq!(solid(&precise), [1, 2, 34, 35, 36, 37, 38]);
    assert!(noted(&precise, "precise_z_height").is_empty());
    assert!(noted(&precise, "top_shell_thickness").is_empty());
}

#[test]
fn shells_come_out_the_same_on_one_thread_and_on_many() {
    let run = |threads: usize| {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        pool.install(|| {
            gcode(&sliced(
                7.95,
                0.16,
                &[
                    ("top_shell_layers", "5"),
                    ("top_shell_thickness", "1.0"),
                    ("bottom_shell_layers", "3"),
                    ("bottom_shell_thickness", "0.8"),
                    ("precise_z_height", "1"),
                ],
            ))
        })
    };
    assert!(
        run(1) == run(8),
        "the file differs with the number of threads"
    );
}
