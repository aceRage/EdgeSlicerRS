//! A prime tower that leaves out the layers with no tool change
//! (`wipe_tower_no_sparse_layers`): it stands only as tall as the layers
//! it is laid on, the nozzle goes down to it there and nothing it does down
//! there crosses the print, and a plate where the toolhead would meet the
//! print is refused with the reference's words.

use std::path::{Path, PathBuf};

use edge_config::{Kind, Settings, Vendor};
use edge_engine::{Chosen, Model, Overrides, Unwatched};
use edge_mesh::{Mesh, primitives::cuboid};

fn reference(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
}

/// A post 20 mm square and 14 mm tall in filament 1 with two bands of
/// filament 2 round it, 3 to 4 mm and 9 to 10 mm up (tools/make_fixture.py
/// `bands`): tool changes on the bands' layers only. Each piece is its own
/// part, a hair apart from the next where no layer is cut, in the middle
/// of the bed.
fn bands(dir: &Path) -> Model {
    let pieces = [
        (0.0, 3.0),
        (3.0, 4.0),
        (4.0, 9.0),
        (9.0, 10.0),
        (10.0, 14.0),
    ];
    let soup = pieces.iter().enumerate().flat_map(|(index, (low, high))| {
        let low = if index == 0 { *low } else { low + 0.02 };
        cuboid([125.5, 126.0, low], [145.5, 146.0, *high])
    });
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join("bands.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(None, object).unwrap();
    let parts = model.objects()[0].shape().part_ids().to_vec();
    assert_eq!(parts.len(), 5);
    // The parts in the order they stand, from the bottom.
    let mut by_height: Vec<(f32, u64)> = (model.objects()[0].shape().parts().iter())
        .zip(&parts)
        .map(|(part, id)| (part.mesh.bounds().unwrap().0[2], *id))
        .collect();
    by_height.sort_by(|a, b| a.0.total_cmp(&b.0));
    let banded = [by_height[1].1, by_height[3].1];
    model
        .set_part_filament(None, object, &banded, Some(2))
        .unwrap();
    model
}

/// A Snapmaker U1 with two filaments of PLA, its presets as shipped.
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
        filaments: vec![filament.clone(), filament],
    }
}

fn slice(model: &Model, set: &str) -> Result<String, String> {
    let overrides = Overrides {
        settings: Settings::from_json(set).unwrap(),
        ..Overrides::default()
    };
    let job = edge_engine::prepare(model.place(None).unwrap(), &u1(), &overrides).unwrap();
    let sliced = edge_engine::slice(job, &Unwatched).map_err(|error| error.to_string())?;
    let gcode = sliced
        .gcode("2001-01-01 at 00:00:00")
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8(gcode.text).unwrap())
}

/// One move as the file has it: where from and to, the nozzle's height,
/// the top of the layer being printed, and whether it laid the tower.
struct Move {
    from: [f64; 2],
    to: [f64; 2],
    z: f64,
    layer_top: f64,
    tower: bool,
    /// Whether it is a wall or fill of the part.
    part: bool,
    extruding: bool,
}

fn moves(gcode: &str) -> Vec<Move> {
    let (mut at, mut z, mut top, mut relative) = ([0.0f64; 2], 0.0f64, 0.0f64, false);
    let mut feature = String::new();
    let mut out = Vec::new();
    for line in gcode.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix(";Z:") {
            top = value.parse().unwrap();
            continue;
        }
        if let Some(name) = line.strip_prefix(";TYPE:") {
            feature = name.to_owned();
            continue;
        }
        let code = line.split(';').next().unwrap_or("");
        let mut words = code.split_whitespace();
        match words.next() {
            Some("G90") => relative = false,
            Some("G91") => relative = true,
            Some("G0" | "G1") => {
                let word = |letter: char| {
                    code.split_whitespace()
                        .skip(1)
                        .find_map(|word| word.strip_prefix(letter)?.parse::<f64>().ok())
                };
                if let Some(to) = word('Z') {
                    z = if relative { z + to } else { to };
                }
                if relative {
                    continue;
                }
                let to = [word('X').unwrap_or(at[0]), word('Y').unwrap_or(at[1])];
                out.push(Move {
                    from: at,
                    to,
                    z,
                    layer_top: top,
                    tower: feature == "Prime tower",
                    part: feature.contains("wall") || feature.contains("infill"),
                    // Pushing filament back without moving lays nothing.
                    extruding: word('E').is_some_and(|e| e > 0.0) && to != at,
                });
                at = to;
            }
            _ => {}
        }
    }
    out
}

/// Whether a straight move from `a` to `b` crosses the box `low`..`high`.
fn crosses(a: [f64; 2], b: [f64; 2], low: [f64; 2], high: [f64; 2]) -> bool {
    // Liang-Barsky clipping of the segment against the box.
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = [b[0] - a[0], b[1] - a[1]];
    for axis in 0..2 {
        for (p, q) in [
            (-d[axis], a[axis] - low[axis]),
            (d[axis], high[axis] - a[axis]),
        ] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
    }
    t0 <= t1
}

const ON: &str = r#"{"wipe_tower_no_sparse_layers": "1"}"#;

#[test]
fn the_tower_stands_as_tall_as_the_layers_it_is_laid_on() {
    let dir = tempfile::tempdir().unwrap();
    let model = bands(dir.path());
    let gcode = slice(&model, ON).unwrap();
    let moves = moves(&gcode);
    // The tower's lines, by the layer they are laid on and their height.
    let mut laid: Vec<(f64, f64)> = Vec::new();
    for laying in moves
        .iter()
        .filter(|laying| laying.tower && laying.extruding)
    {
        if laid.last().is_none_or(|(top, _)| *top != laying.layer_top) {
            laid.push((laying.layer_top, laying.z));
        }
        // All of a layer's tower at the one height.
        assert_eq!(laid.last().unwrap().1, laying.z);
    }
    // On the four layers the bands start and end on, one 0.2 mm tower
    // layer on another, as the reference lays them.
    assert_eq!(
        laid,
        vec![(3.25, 0.2), (4.25, 0.4), (9.25, 0.6), (10.25, 0.8)]
    );
}

#[test]
fn down_on_the_tower_the_nozzle_keeps_out_of_the_print() {
    let dir = tempfile::tempdir().unwrap();
    let model = bands(dir.path());
    let gcode = slice(&model, ON).unwrap();
    let moves = moves(&gcode);
    // The post, from its walls and fill, and half a line round.
    let (mut low, mut high) = ([f64::MAX; 2], [f64::MIN; 2]);
    for laying in moves
        .iter()
        .filter(|laying| laying.extruding && laying.part)
    {
        if laying.layer_top > 0.0 && laying.layer_top < 14.1 {
            for point in [laying.from, laying.to] {
                low = [low[0].min(point[0]), low[1].min(point[1])];
                high = [high[0].max(point[0]), high[1].max(point[1])];
            }
        }
    }
    let (low, high) = (
        [low[0] - 0.25, low[1] - 0.25],
        [high[0] + 0.25, high[1] + 0.25],
    );
    // Every move made lower than the layer being printed, travel and
    // extrusion alike, keeps clear of the post: the post stands to the
    // layer below already.
    let below: Vec<&Move> = moves
        .iter()
        .filter(|laying| laying.layer_top > 0.0 && laying.z < laying.layer_top - 0.21)
        .collect();
    assert!(below.len() > 100, "the tower is laid below the layers");
    for laying in &below {
        assert!(
            !crosses(laying.from, laying.to, low, high),
            "a move at {} mm on the layer up to {} mm crosses the post: {:?} to {:?}",
            laying.z,
            laying.layer_top,
            laying.from,
            laying.to
        );
    }
    // And the nozzle goes down to the tower and up off it straight, not
    // on a slope that would cross the post on its way.
    for pair in moves.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        if (after.z - before.z).abs() > 1.0 {
            assert_eq!(
                after.from, after.to,
                "from {} to {} mm in place",
                before.z, after.z
            );
        }
    }
}

#[test]
fn a_tower_beside_the_print_is_refused_and_one_clear_of_it_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let model = bands(dir.path());
    // Where the U1 puts it, far from the post in the middle of the bed.
    assert!(slice(&model, ON).is_ok());
    // Beside the post, within the head's radius of it: the head would
    // come down on it.
    let beside = r#"{"wipe_tower_no_sparse_layers": "1", "wipe_tower_x": ["160"],
        "wipe_tower_y": ["126"]}"#;
    let refused = slice(&model, beside).unwrap_err();
    assert_eq!(refused, edge_tower::CLEARANCE_ERROR);
    // The same with the tower laid on every layer is no matter.
    assert!(
        slice(
            &model,
            r#"{"wipe_tower_x": ["160"], "wipe_tower_y": ["126"]}"#
        )
        .is_ok()
    );
}

#[test]
fn with_the_setting_off_nothing_changes() {
    let dir = tempfile::tempdir().unwrap();
    let model = bands(dir.path());
    let off = slice(&model, r#"{"wipe_tower_no_sparse_layers": "0"}"#).unwrap();
    let plain = slice(&model, "{}").unwrap();
    assert_eq!(off, plain);
    // The tower stands on every layer up to the last change, at the layer.
    let moves = moves(&off);
    assert!(
        moves
            .iter()
            .filter(|laying| laying.tower && laying.extruding)
            .all(|laying| (laying.z - laying.layer_top).abs() < 1e-6)
    );
}
