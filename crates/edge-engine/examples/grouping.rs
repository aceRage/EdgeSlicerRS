//! The filament grouping of a two-nozzle printer (the H2D) against the
//! reference's G-code, while the grouping does not drive the output yet.
//!
//! `cargo run --release -p edge-engine --example grouping -- PROJECT.3mf
//! [REFERENCE.gcode] [--plate N] [--layers]` slices the project as `edge
//! slice` does as far as the tool order, and prints which nozzle each
//! filament prints from (`filament_map`, extruders from 1), the same
//! worked out with the flush matrix read the reference's way (see
//! `edge_group::prepare_flush_matrices`), and each layer's filament order
//! with each filament's nozzle. Given the reference's G-code it compares:
//! `filament_map` with the file's config block (the map the reference's
//! grouping wrote back), and each layer's order with the filaments the
//! file's layer prints parts with, in turn (`T` changes, prime tower
//! left out). The last line sums it up for a table.

use std::path::PathBuf;
use std::sync::Mutex;

use edge_engine::{Arrange, Chosen, Grouped, Overrides, Watcher};

#[derive(Default)]
struct Catch(Mutex<Option<Grouped>>);

impl Watcher for Catch {
    fn grouped(&self, grouped: &Grouped) {
        *self.0.lock().unwrap() = Some(grouped.clone());
    }
}

/// The reference's file: its `filament_map` and each layer's filaments in
/// the order they print parts.
fn reference(text: &str) -> (Vec<i32>, Vec<Vec<u32>>) {
    let mut map = Vec::new();
    let mut layers: Vec<Vec<u32>> = Vec::new();
    let mut tool: Option<u32> = None;
    let mut feature = String::new();
    let mut in_tower = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(values) = line.strip_prefix("; filament_map = ") {
            map = values
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            continue;
        }
        if line == "; CHANGE_LAYER" {
            layers.push(Vec::new());
            continue;
        }
        if line == "; WIPE_TOWER_START" {
            in_tower = true;
        } else if line == "; WIPE_TOWER_END" {
            in_tower = false;
        }
        if let Some(name) = line.strip_prefix("; FEATURE: ") {
            feature = name.to_owned();
            continue;
        }
        let code = line.split(';').next().unwrap_or("").trim();
        let mut words = code.split_whitespace();
        let Some(word) = words.next() else {
            continue;
        };
        if let Some(number) = word.strip_prefix('T').and_then(|n| n.parse::<u32>().ok()) {
            if number < 100 {
                tool = Some(number);
            }
            continue;
        }
        if !matches!(word, "G1" | "G2" | "G3")
            || in_tower
            || feature == "Prime tower"
            || feature == "Custom"
        {
            continue;
        }
        let extrudes = words.any(|w| {
            w.strip_prefix('E')
                .and_then(|e| e.parse::<f64>().ok())
                .is_some_and(|e| e > 0.0)
        });
        if let (true, Some(tool), Some(layer)) = (extrudes, tool, layers.last_mut())
            && layer.last() != Some(&tool)
        {
            layer.push(tool);
        }
    }
    (map, layers)
}

fn joined<T: ToString>(items: &[T]) -> String {
    items
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut args = std::env::args().skip(1);
    let usage = "PROJECT.3mf [REFERENCE.gcode] [--plate N] [--layers]";
    let project = PathBuf::from(args.next().ok_or(usage)?);
    let mut gcode: Option<PathBuf> = None;
    let mut plate = None;
    let mut every_layer = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--plate" => plate = Some(args.next().ok_or("--plate N")?.parse::<u32>()?),
            "--layers" => every_layer = true,
            _ if gcode.is_none() => gcode = Some(PathBuf::from(arg)),
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
    let watch = Catch::default();
    let then = match edge_engine::slice(job, &watch) {
        Ok(_) => "the slice went on to write G-code".to_owned(),
        Err(error) => format!("the slice then stopped: {error}"),
    };
    let Some(grouped) = watch.0.into_inner().unwrap() else {
        println!("no grouping: {then}");
        return Ok(());
    };
    println!("{then}");
    let Some(grouping) = &grouped.grouping else {
        println!("nothing to group");
        return Ok(());
    };
    let ours = grouping.filament_map.clone();
    println!(
        "mode {:?}, used filaments (from 0) {}",
        grouped.input.mode,
        joined(&grouping.result.used)
    );
    println!("filament_map (ours)                   {}", joined(&ours));
    // The reference's own reading of the flush matrix.
    let mut as_reference = grouped.input.clone();
    as_reference.flush = edge_group::prepare_flush_matrices(
        &grouped.flush_values,
        grouped.machine.extruders,
        grouped.input.filament_count,
        grouped.flush_multiplier,
        grouped.flush_fast.as_deref(),
        true,
    )?;
    let theirs_way = edge_group::group_filaments(&grouped.machine, &as_reference)?
        .map(|grouping| grouping.filament_map)
        .unwrap_or_default();
    println!(
        "filament_map (matrix read as the reference reads it) {}",
        joined(&theirs_way)
    );

    let nozzle_of = |filament: u32| {
        grouping
            .result
            .nozzle_for(i64::from(filament))
            .map_or(-1, |nozzle| nozzle.group_id)
    };
    let (reference_map, reference_layers) = match &gcode {
        Some(path) => {
            let (map, layers) = reference(&std::fs::read_to_string(path)?);
            (Some(map), layers)
        }
        None => (None, Vec::new()),
    };
    if let Some(map) = &reference_map {
        println!(
            "filament_map (reference's file)       {}  {}",
            joined(map),
            if *map == ours { "same" } else { "DIFFERENT" }
        );
    }
    let mut same_layers = 0;
    let layers = grouped.orders.len().max(reference_layers.len());
    for index in 0..layers {
        let order = grouped.orders.get(index).cloned().unwrap_or_default();
        let nozzles: Vec<i32> = order.iter().map(|f| nozzle_of(*f)).collect();
        let theirs = reference_layers.get(index);
        let same = theirs == Some(&order);
        same_layers += usize::from(same);
        if every_layer || (gcode.is_some() && !same) {
            println!(
                "layer {index:3}: ours {} on nozzles {}{}",
                joined(&order),
                joined(&nozzles),
                theirs.map_or(String::new(), |theirs| format!(
                    "  reference {}{}",
                    joined(theirs),
                    if same { "" } else { "  DIFFERENT" }
                ))
            );
        }
    }
    println!(
        "RESULT map {} as-reference-map {} layers {}/{} (ours {}, reference {})",
        match &reference_map {
            Some(map) if *map == ours => "same",
            Some(_) => "DIFFERENT",
            None => "-",
        },
        match &reference_map {
            Some(map) if *map == theirs_way => "same",
            Some(_) => "DIFFERENT",
            None => "-",
        },
        same_layers,
        layers,
        grouped.orders.len(),
        reference_layers.len()
    );
    Ok(())
}
