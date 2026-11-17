//! `edge slice` arranges a model without plates as the reference's command
//! line does, and `--arrange 0` leaves it as the file has it.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// A binary STL of a box `size` mm with a corner at `at`.
fn write_box(path: &Path, at: [f32; 3], size: [f32; 3]) {
    let [x0, y0, z0] = at;
    let [x1, y1, z1] = [at[0] + size[0], at[1] + size[1], at[2] + size[2]];
    let v = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    let t: [[usize; 3]; 12] = [
        [0, 2, 1],
        [0, 3, 2],
        [4, 5, 6],
        [4, 6, 7],
        [0, 1, 5],
        [0, 5, 4],
        [1, 2, 6],
        [1, 6, 5],
        [2, 3, 7],
        [2, 7, 6],
        [3, 0, 4],
        [3, 4, 7],
    ];
    let mut bytes = vec![0u8; 80];
    bytes.extend_from_slice(&(t.len() as u32).to_le_bytes());
    for tri in t {
        bytes.extend_from_slice(&[0u8; 12]);
        for corner in tri {
            for value in v[corner] {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&[0u8; 2]);
    }
    std::fs::write(path, bytes).unwrap();
}

/// The first layer's extruded extent, `[min_x, min_y, max_x, max_y]`.
fn first_layer(gcode: &str) -> [f64; 4] {
    let mut extent = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    let mut layer = 0;
    for line in gcode.lines() {
        if line.starts_with("; layer num/total_layer_count:") {
            layer += 1;
            continue;
        }
        if layer != 1 || !line.starts_with("G1 ") {
            continue;
        }
        let word = |letter: char| {
            line.split_whitespace()
                .find_map(|w| w.strip_prefix(letter)?.parse::<f64>().ok())
        };
        if let (Some(x), Some(y), Some(e)) = (word('X'), word('Y'), word('E'))
            && e > 0.0
        {
            extent = [
                extent[0].min(x),
                extent[1].min(y),
                extent[2].max(x),
                extent[3].max(y),
            ];
        }
    }
    extent
}

fn slice(model: &Path, gcode: &Path, extra: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_edge"))
        .arg("slice")
        .arg(model)
        .args(["--vendor", "BBL", "--machine", "Bambu Lab A1 0.4 nozzle"])
        .args(["--process", "0.20mm Standard @BBL A1"])
        .args(["--filament", "Bambu PLA Basic @BBL A1"])
        .arg("--profiles")
        .arg(reference("resources/profiles"))
        .args(extra)
        .arg("--gcode")
        .arg(gcode)
        .env("SOURCE_DATE_EPOCH", "1")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{said}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    said
}

#[test]
fn an_stl_is_arranged_as_the_reference_arranges_it() {
    let dir = tempfile::tempdir().unwrap();
    let stl = dir.path().join("box.stl");
    write_box(&stl, [10.0, 20.0, 0.0], [25.0, 20.0, 15.0]);
    let gcode = dir.path().join("box.gcode");

    // The reference's command line puts it in the middle of the A1's
    // 256 mm bed, turned to run along Y (the A1 is an i3): its first
    // layer reaches x 118.325..137.675, y 115.825..140.175.
    let said = slice(&stl, &gcode, &[]);
    assert!(said.contains("arranged on the bed"), "{said}");
    let at = first_layer(&std::fs::read_to_string(&gcode).unwrap());
    let expected = [118.325, 115.825, 137.675, 140.175];
    assert!(
        at.iter()
            .zip(&expected)
            .all(|(a, b)| (a - b).abs() < 0.0015),
        "{at:?}"
    );

    // Not arranged, it is only moved to the middle, as it was.
    let said = slice(&stl, &gcode, &["--arrange", "0"]);
    assert!(!said.contains("arranged on the bed"), "{said}");
    let at = first_layer(&std::fs::read_to_string(&gcode).unwrap());
    let expected = [115.825, 118.325, 140.175, 137.675];
    assert!(
        at.iter()
            .zip(&expected)
            .all(|(a, b)| (a - b).abs() < 0.0015),
        "{at:?}"
    );
}
