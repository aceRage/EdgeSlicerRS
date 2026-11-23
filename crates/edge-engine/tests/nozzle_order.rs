//! The order of filaments within a layer on a printer of two extruders
//! whose filaments the grouping places (a Bambu Lab H2D): nozzle by
//! nozzle, each nozzle from the filament it last held (Bambu Studio's
//! `reorder_filaments_for_multi_nozzle_extruder`, as C++ fix PR
//! aceRage/EdgeSlicer#1169 orders the layers; the pinned reference still
//! takes one chain over the whole layer).

use std::path::{Path, PathBuf};

use edge_config::{Kind, Settings, Vendor};
use edge_engine::{Chosen, Model, Overrides, Unwatched};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// Three blocks 12 × 12 mm in a row, 1 mm tall, in filaments 1 to 3 from
/// the left: parts of one object, so every layer prints all three.
fn three_blocks(dir: &Path) -> Model {
    let soup = (0..3).flat_map(|at| {
        let x = 150.0 + 20.0 * at as f32;
        cuboid([x, 150.0, 0.0], [x + 12.0, 162.0, 1.0])
    });
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join("blocks.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(None, object).unwrap();
    let shape = model.objects()[0].shape().clone();
    let left = |at: usize| {
        let part = &shape.parts()[at];
        part.mesh
            .vertices
            .iter()
            .map(|vertex| part.transform.apply(vertex.map(f64::from))[0])
            .fold(f64::INFINITY, f64::min)
    };
    let mut by_place: Vec<usize> = (0..shape.parts().len()).collect();
    by_place.sort_by(|a, b| left(*a).total_cmp(&left(*b)));
    assert_eq!(by_place.len(), 3);
    for (filament, at) in by_place.into_iter().enumerate() {
        let part = shape.part_ids()[at];
        model
            .set_part_filament(None, object, &[part], Some(filament + 1))
            .unwrap();
    }
    model
}

fn h2d() -> Chosen {
    let vendor = Vendor::open(&reference("resources/profiles"), "BBL").unwrap();
    let preset = |kind, name: &str| (name.to_owned(), vendor.resolve(kind, name).unwrap());
    let filament = preset(Kind::Filament, "Bambu PLA Basic @BBL H2D");
    Chosen {
        machine: Some(preset(Kind::Machine, "Bambu Lab H2D 0.4 nozzle")),
        process: Some(preset(Kind::Process, "0.20mm Standard @BBL H2D")),
        filaments: vec![filament; 3],
    }
}

/// The filaments (from 0) each layer takes up, in turn, the tool in hand
/// at a layer's start not counted again.
fn layer_tools(model: &Model, set: &str) -> Vec<Vec<u32>> {
    let overrides = Overrides {
        settings: Settings::from_json(set).unwrap(),
        ..Overrides::default()
    };
    let job = edge_engine::prepare(model.place(None).unwrap(), &h2d(), &overrides).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let text = String::from_utf8(sliced.gcode("2001-01-01 at 00:00:00").unwrap().text).unwrap();
    let mut layers: Vec<Vec<u32>> = Vec::new();
    for line in text.lines() {
        if line.starts_with("; CHANGE_LAYER") {
            layers.push(Vec::new());
        } else if let Some(rest) = line.strip_prefix('T')
            && let Some(layer) = layers.last_mut()
            && let Ok(tool) = rest.split_whitespace().next().unwrap_or("").parse::<u32>()
            && tool < 3
        {
            layer.push(tool);
        }
    }
    layers
}

#[test]
fn each_nozzle_is_ordered_from_the_filament_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let model = three_blocks(dir.path());
    // Filaments 1 and 2 on the left, 3 on the right. From 3 the chain over
    // the whole layer takes 3, 1, 2 (10 + 500 by this matrix), charging
    // 3 -> 1 though the right nozzle keeps 3, and flushes 2 -> 1 -> 2 on
    // the left (600 mm³); nozzle by nozzle the layer is 3, then 2, 1 on the
    // left (100 mm³), and the next layer goes on from 1 on the left.
    let set = r##"{
        "filament_colour": ["#FF0000", "#00FF00", "#0000FF"],
        "filament_map": ["1", "1", "2"],
        "filament_map_mode": "Manual",
        "flush_volumes_matrix": ["0", "500", "300", "100", "0", "300", "10", "1000", "0"],
        "flush_multiplier": ["1"]
    }"##;
    let layers = layer_tools(&model, set);
    assert_eq!(layers.len(), 5, "{layers:?}");
    // The tool in hand at a layer's start is not taken up again: the first
    // layer (1, 2, 3, filament 1 taken up before it) ends on 3, so the
    // second takes up 2, 1 and ends on the left; the third goes on from 1
    // on the left: 2, then 3. The chain would take 1, 2 on the second.
    assert_eq!(layers[1], [1, 0], "{layers:?}");
    assert_eq!(layers[2], [1, 2], "{layers:?}");
    // No layer turns back to an extruder it left.
    let extruder = |tool: u32| if tool == 2 { 1 } else { 0 };
    let mut in_hand = Some(0);
    for (index, layer) in layers.iter().enumerate() {
        let mut taken: Vec<u32> = in_hand.into_iter().collect();
        taken.extend(layer.iter().copied());
        let switches = (taken.windows(2))
            .filter(|pair| extruder(pair[0]) != extruder(pair[1]))
            .count();
        assert!(switches <= 1, "layer {index}: {layers:?}");
        in_hand = taken.last().copied();
    }
}
