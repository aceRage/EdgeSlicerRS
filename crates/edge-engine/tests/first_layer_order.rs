//! The order of tools on the first layer: the tool whose smallest piece
//! is largest goes first (ToolOrdering.cpp
//! `generate_first_layer_tool_order`), its pieces measured as the
//! reference keeps them for it, after elephant foot compensation
//! (`raw_slices`, PrintObjectSlice.cpp `PrintObject::slice`).

use std::path::{Path, PathBuf};

use edge_config::{Kind, Settings, Vendor};
use edge_engine::{Chosen, Model, Overrides, Unwatched};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// Four blocks 10 × 20 mm in a row, 8, 6, 4 and 2 mm tall, in filaments 1
/// to 4 from the left: parts of one object, 1/32 mm apart, a gap the
/// slicing closes before compensating, so the four are shrunk as one.
/// Every corner and middle is exact in single precision, and the middle of
/// the row a whole number of units, so the four are
/// cut the same to the unit in their object's frame.
fn four_blocks(dir: &Path) -> Model {
    let heights = [8.0, 6.0, 4.0, 2.0];
    let soup = heights.iter().enumerate().flat_map(|(at, height)| {
        let x = 115.0 + 10.031_25 * at as f32;
        cuboid([x, 126.0, 0.0], [x + 10.0, 146.0, *height])
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
    assert_eq!(by_place.len(), 4);
    for (filament, at) in by_place.into_iter().enumerate() {
        let part = shape.part_ids()[at];
        model
            .set_part_filament(None, object, &[part], Some(filament + 1))
            .unwrap();
    }
    model
}

fn u1() -> Chosen {
    let vendor = Vendor::open(&reference("resources/profiles"), "Snapmaker").unwrap();
    let preset = |kind, name: &str| (name.to_owned(), vendor.resolve(kind, name).unwrap());
    let filament = preset(Kind::Filament, "Generic PLA @U1 0.4 nozzle");
    Chosen {
        machine: Some(preset(Kind::Machine, "Snapmaker U1 (0.4 nozzle)")),
        process: Some(preset(
            Kind::Process,
            "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
        )),
        filaments: vec![filament; 4],
    }
}

/// The tools the G-code takes up to the end of its first layer, in turn.
fn first_layer_tools(model: &Model, set: &str) -> Vec<u32> {
    let overrides = Overrides {
        settings: Settings::from_json(set).unwrap(),
        ..Overrides::default()
    };
    let job = edge_engine::prepare(model.place(None).unwrap(), &u1(), &overrides).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).unwrap();
    let text = String::from_utf8(sliced.gcode("2001-01-01 at 00:00:00").unwrap().text).unwrap();
    let mut changes = 0;
    let mut tools: Vec<u32> = Vec::new();
    for line in text.lines() {
        if line.starts_with(";LAYER_CHANGE") {
            changes += 1;
            if changes > 1 {
                break;
            }
        }
        if let Some(tool) = line.strip_prefix('T').and_then(|t| t.trim().parse().ok())
            && tools.last() != Some(&tool)
        {
            tools.push(tool);
        }
    }
    tools
}

#[test]
fn the_blocks_at_the_ends_lose_more_to_elephant_foot_and_go_last() {
    let dir = tempfile::tempdir().unwrap();
    let model = four_blocks(dir.path());
    // Elephant foot compensation takes 0.15 mm off the outline of the
    // row: the middle blocks keep 10 × 19.7 mm, the end ones 9.85 × 19.7.
    // Of two the same, the lower filament first: the reference's order on
    // this plate.
    assert_eq!(
        first_layer_tools(&model, r#"{"elefant_foot_compensation": "0.15"}"#),
        [1, 2, 0, 3]
    );
    // Without it the four are the same in the file, but not as cut: each
    // is cut in single precision about its object's middle, and the ends
    // come out larger by a few square units. The pinned build gives
    // 0, 3, 1, 2 on this plate too (the same blocks written as a project
    // with the U1's settings, elephant foot compensation 0).
    assert_eq!(
        first_layer_tools(&model, r#"{"elefant_foot_compensation": "0"}"#),
        [0, 3, 1, 2]
    );
}
