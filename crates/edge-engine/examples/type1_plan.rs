//! A check of the rewritten Type 1 tower's planner (edge-tower `type1`)
//! against the reference's G-code, while the planner does not drive the
//! output yet.
//!
//! `cargo run --release -p edge-engine --example type1_plan -- PROJECT.3mf
//! GCODE [--plate N]` slices the project as `edge slice` does, for the
//! settings it resolves and the tower's place, reads the layers and the
//! tool changes from GCODE (the reference's file, so that the planner is
//! checked on the reference's own tool order), plans the tower and prints
//! it as JSON: the footprint, the first layer's and every layer's box (of
//! the lines' middles, as a G-code reader sees them), the layers laid, the
//! top, and each layer's depth.

use std::path::PathBuf;

use edge_engine::{Arrange, Chosen, Overrides, Unwatched};
use edge_tower::type1;

/// The layers of a G-code file as the tower sees them: each one's top and
/// height, and the tool changes made on it, from the tool in hand.
fn layers_of(text: &str) -> (Vec<type1::LayerIn>, Option<usize>) {
    let mut layers: Vec<type1::LayerIn> = Vec::new();
    let mut tool: Option<usize> = None;
    let mut first_tool = None;
    let mut started = false;
    for line in text.lines() {
        let line = line.trim();
        if line == "; CHANGE_LAYER" || line == ";LAYER_CHANGE" {
            started = true;
            layers.push(type1::LayerIn {
                z: 0.0,
                height: 0.0,
                changes: Vec::new(),
            });
            continue;
        }
        if let Some(layer) = layers.last_mut() {
            let value = |prefix: &str| {
                line.strip_prefix(prefix)
                    .and_then(|rest| rest.trim().parse::<f32>().ok())
            };
            if let Some(z) = value("; Z_HEIGHT:").or_else(|| value(";Z:"))
                && layer.z == 0.0
            {
                layer.z = z;
            }
            if let Some(height) = value("; LAYER_HEIGHT:").or_else(|| value(";HEIGHT:"))
                && layer.height == 0.0
            {
                layer.height = height;
            }
        }
        let code = line.split(';').next().unwrap_or("").trim();
        let Some(word) = code.split_whitespace().next() else {
            continue;
        };
        let Some(number) = word.strip_prefix('T').and_then(|n| n.parse::<usize>().ok()) else {
            continue;
        };
        if number >= 100 {
            continue;
        }
        if started
            && let (Some(old), Some(layer)) = (tool, layers.last_mut())
            && old != number
        {
            layer.changes.push(type1::ChangeIn {
                old,
                new: number,
                purge_volume: 0.0,
                // Read from a G-code file of one nozzle.
                nozzle_change: None,
            });
        }
        if first_tool.is_none() {
            first_tool = Some(number);
        }
        tool = Some(number);
    }
    // The tower stands up to the last layer with a change.
    let last = layers.iter().rposition(|layer| !layer.changes.is_empty());
    layers.truncate(last.map_or(0, |last| last + 1));
    (layers, first_tool)
}

/// The laid tower as a G-code file holds it, for a G-code reader: each
/// layer's visits moved to the tower's place (GCode.cpp
/// `post_process_wipe_tower_moves`), the change G-code as a bare `T`, a
/// travel to where each visit and each purge starts (`append_tcr`), and a
/// line of something else before each visit, as the objects' lines are in
/// a whole file.
fn pseudo_gcode(plan: &type1::Plan, visits: &[Vec<type1::Visit>]) -> String {
    let (ox, oy) = plan.origin;
    let mut out = format!("M83\nT{}\n", plan.start_tool);
    let moved = |line: &str| -> String {
        let mut words: Vec<String> = Vec::new();
        for word in line.split_whitespace() {
            let value = |w: &str| w[1..].parse::<f32>().unwrap_or(0.0);
            match word.as_bytes()[0] {
                b'X' => words.push(format!("X{:.3}", value(word) + ox)),
                b'Y' => words.push(format!("Y{:.3}", value(word) + oy)),
                _ => words.push(word.to_owned()),
            }
        }
        words.join(" ")
    };
    for (index, layer) in visits.iter().enumerate() {
        if layer.is_empty() {
            continue;
        }
        if plan.skip_sparse && plan.layers[index].changes.is_empty() {
            continue;
        }
        let planned = &plan.layers[index];
        out += &format!(
            "; CHANGE_LAYER\n; Z_HEIGHT: {}\n; LAYER_HEIGHT: {}\n",
            planned.z, planned.height
        );
        for visit in layer {
            out += "; FEATURE: Outer wall\nG1 X1 Y1 F6000\nG1 X1.5 Y1 E0.01\n";
            out += &format!(
                "G1 X{:.3} Y{:.3}\n",
                visit.start_pos.0 + ox,
                visit.start_pos.1 + oy
            );
            for line in visit.gcode.lines() {
                if line == "[change_filament_gcode]" {
                    out += &format!("T{}\n", visit.new_tool);
                    out += &format!(
                        "G1 X{:.3} Y{:.3}\n",
                        visit.tool_change_start_pos.0 + ox,
                        visit.tool_change_start_pos.1 + oy
                    );
                } else if line.starts_with("G1 ")
                    || line.starts_with("G2 ")
                    || line.starts_with("G3 ")
                {
                    out += &moved(line);
                    out += "\n";
                } else if !line.starts_with('[') {
                    out += line;
                    out += "\n";
                }
            }
        }
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let project = PathBuf::from(args.next().ok_or("PROJECT.3mf GCODE [--plate N]")?);
    let gcode = PathBuf::from(args.next().ok_or("PROJECT.3mf GCODE [--plate N]")?);
    let mut plate = None;
    let mut lines_out: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plate" => plate = Some(args.next().ok_or("--plate N")?.parse::<u32>()?),
            "--lines" => lines_out = Some(PathBuf::from(args.next().ok_or("--lines OUT")?)),
            _ => return Err(format!("unknown argument {arg}").into()),
        }
    }
    let overrides = Overrides::default();
    let chosen = Chosen::default();
    let opened = edge_engine::open_to_slice(
        &project,
        plate,
        Arrange::from_option(2),
        &chosen,
        &overrides,
    )?;
    let job = edge_engine::prepare(opened.loaded, &chosen, &overrides)?;
    let sliced = edge_engine::slice(job, &Unwatched)?;
    let plate_index = plate.map_or(0, |plate| plate as usize - 1);
    let mut settings = type1::Settings::from_settings(&sliced.settings, plate_index)?;
    // Where the pipeline put the tower (it moves one whose place no longer
    // fits it onto the bed).
    settings.x = sliced.tower_settings.x as f32;
    settings.y = sliced.tower_settings.y as f32;
    // The file gives every position taken back by the first tool's
    // `extruder_offset` (GCode.cpp `point_to_gcode`): 0x2 on the P1S and X1C.
    let offset: (f32, f32) = sliced
        .settings
        .texts("extruder_offset")
        .and_then(|offsets| offsets.first())
        .and_then(|first| first.split_once('x'))
        .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
        .unwrap_or((0.0, 0.0));
    settings.x -= offset.0;
    settings.y -= offset.1;
    let text = std::fs::read_to_string(&gcode)?;
    let (layers, first_tool) = layers_of(&text);
    let plan = type1::plan(&settings, &layers, first_tool.unwrap_or(0))?;
    if let Some(out) = &lines_out {
        let visits = type1::lay(&plan, &settings)?;
        std::fs::write(out, pseudo_gcode(&plan, &visits))?;
    }

    let (count, top) = plan.laid_layers();
    let mut all = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut first = None;
    let mut per_layer = Vec::new();
    for index in 0..plan.layers.len() {
        if !plan.laid(index) {
            continue;
        }
        let extent = plan.layer_extent(index)?;
        all = [
            all[0].min(extent[0]),
            all[1].min(extent[1]),
            all[2].max(extent[2]),
            all[3].max(extent[3]),
        ];
        first.get_or_insert(extent);
        per_layer.push(format!(
            "{{\"z\": {:.3}, \"depth\": {:.4}, \"y_shift\": {:.4}, \"changes\": {}, \"box\": [{:.4}, {:.4}, {:.4}, {:.4}]}}",
            plan.layers[index].z,
            plan.layers[index].depth,
            plan.y_shifts[index],
            plan.layers[index].changes.len(),
            extent[0],
            extent[1],
            extent[2],
            extent[3]
        ));
    }
    let first = first.unwrap_or(all);
    let categories: Vec<String> = plan
        .blocks
        .iter()
        .map(|block| {
            format!(
                "[{}, {:.4}, {:.4}]",
                block.category, block.start_depth, block.depth
            )
        })
        .collect();
    let (fx, fy) = plan.footprint();
    // Where the first layer's purge rows run: from one line inside the
    // wall to the other, the first at the first block's start.
    let (ox, oy) = (
        plan.origin.0 + plan.rib_offset.0,
        plan.origin.1
            + plan.rib_offset.1
            + plan.y_shifts.get(plan.first_layer).copied().unwrap_or(0.0),
    );
    let rows = [
        ox + plan.line_width,
        ox + plan.width - plan.line_width,
        oy + plan.blocks.first().map_or(0.0, |block| block.start_depth),
    ];
    println!(
        "{{\"width\": {:.4}, \"depth\": {:.4}, \"height\": {:.3}, \"extra_spacing\": {:.6}, \
         \"brim_width\": {:.4}, \"footprint\": [{fx:.4}, {fy:.4}], \"rib_offset\": [{:.4}, {:.4}], \
         \"rib\": {}, \"blocks\": [{}], \"layers\": {count}, \"top\": {top:.3}, \
         \"first_box\": [{:.4}, {:.4}, {:.4}, {:.4}], \"box\": [{:.4}, {:.4}, {:.4}, {:.4}], \
         \"origin\": [{:.4}, {:.4}], \"rows\": [{:.4}, {:.4}, {:.4}], \"per_layer\": [{}]}}",
        plan.width,
        plan.depth,
        plan.height,
        plan.extra_spacing,
        plan.brim_width,
        plan.rib_offset.0,
        plan.rib_offset.1,
        plan.ribs.map_or("null".to_owned(), |ribs| format!(
            "[{:.4}, {:.4}]",
            ribs.width, ribs.length
        )),
        categories.join(", "),
        first[0],
        first[1],
        first[2],
        first[3],
        all[0],
        all[1],
        all[2],
        all[3],
        plan.origin.0,
        plan.origin.1,
        rows[0],
        rows[1],
        rows[2],
        per_layer.join(", ")
    );
    Ok(())
}
