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

/// The same post with its two bands in filament 2 on one half only, the
/// half at the larger x: on the bands' layers the filament changes in the
/// middle of the layer, with the nozzle over the post (the reference's
/// test of aceRage/EdgeSlicer#1020, `halves.stl`).
fn halves(dir: &Path) -> Model {
    let mut soup = Vec::new();
    for (index, (low, high, split)) in [
        (0.0, 3.0, false),
        (3.0, 4.0, true),
        (4.0, 9.0, false),
        (9.0, 10.0, true),
        (10.0, 14.0, false),
    ]
    .into_iter()
    .enumerate()
    {
        let low = if index == 0 { low } else { low + 0.02 };
        if split {
            soup.extend(cuboid([125.5, 126.0, low], [135.49, 146.0, high]));
            soup.extend(cuboid([135.51, 126.0, low], [145.5, 146.0, high]));
        } else {
            soup.extend(cuboid([125.5, 126.0, low], [145.5, 146.0, high]));
        }
    }
    let mesh = Mesh::from_triangle_soup(soup);
    let path = dir.join("halves.stl");
    std::fs::write(&path, edge_mesh::stl::to_binary(&mesh)).unwrap();
    let mut model = Model::open(&path).unwrap();
    let object = model.objects()[0].id();
    model.split_to_parts(None, object).unwrap();
    let parts = model.objects()[0].shape().part_ids().to_vec();
    assert_eq!(parts.len(), 7);
    // The halves at the larger x of the bands, by where they start.
    let banded: Vec<u64> = (model.objects()[0].shape().parts().iter())
        .zip(&parts)
        .filter(|(part, _)| part.mesh.bounds().unwrap().0[0] > 135.0)
        .map(|(_, id)| *id)
        .collect();
    assert_eq!(banded.len(), 2);
    model
        .set_part_filament(None, object, &banded, Some(2))
        .unwrap();
    model
}

/// Two filaments' presets on one printer, as shipped.
fn printer(vendor: &str, machine: &str, process: &str, filament: &str) -> Chosen {
    let vendor = Vendor::open(&reference("resources/profiles"), vendor).unwrap();
    let preset = |kind, name: &str| (name.to_owned(), vendor.resolve(kind, name).unwrap());
    let filament = preset(Kind::Filament, filament);
    Chosen {
        machine: Some(preset(Kind::Machine, machine)),
        process: Some(preset(Kind::Process, process)),
        filaments: vec![filament.clone(), filament],
    }
}

/// A Snapmaker U1 with two filaments of PLA, its presets as shipped.
fn u1() -> Chosen {
    printer(
        "Snapmaker",
        "Snapmaker U1 (0.4 nozzle)",
        "0.20mm Standard @Snapmaker U1 (0.4 nozzle)",
        "Generic PLA @U1 0.4 nozzle",
    )
}

fn slice(model: &Model, set: &str) -> Result<String, String> {
    slice_on(model, &u1(), set)
}

fn slice_on(model: &Model, chosen: &Chosen, set: &str) -> Result<String, String> {
    let overrides = Overrides {
        settings: Settings::from_json(set).unwrap(),
        ..Overrides::default()
    };
    let job = edge_engine::prepare(model.place(None).unwrap(), chosen, &overrides).unwrap();
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

/// What [`low_off_the_tower`] finds in a file.
#[derive(Debug, Default)]
struct Lows {
    /// Lines that had the nozzle below the layer, on the tower.
    on_tower: usize,
    /// Lines that had it below the layer anywhere else, as written.
    off_tower: Vec<String>,
}

/// Every line that moves the nozzle, travel, extrusion, or the filament
/// alone (a retraction, a push back), with the nozzle lower than the top
/// of the layer printed last, sorted by whether it is over the prime tower:
/// the footprint of everything laid as tower, grown by `margin`, which a
/// move must start and end in. A new layer's top counts from the first
/// move that reaches it, so the old tool finishing at the last layer's
/// height is not taken for a fall. Reads this engine's files and the
/// reference's alike: arcs end where they say, a relative move
/// (`G91`) moves by what it says.
fn low_off_the_tower(gcode: &str, margin: f64) -> Lows {
    // One pass for the tower's footprint, one for the moves.
    let mut tower = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut lows = Lows::default();
    for pass in 0..2 {
        let (mut at, mut z, mut relative) = ([0.0f64; 2], 0.0f64, false);
        let (mut top, mut coming): (Option<f64>, Option<f64>) = (None, None);
        let mut on_tower = false;
        for line in gcode.lines() {
            let line = line.trim();
            let tag = line
                .strip_prefix(";Z:")
                .or_else(|| line.strip_prefix("; Z_HEIGHT:"));
            if let Some(value) = tag {
                coming = value.trim().parse().ok();
                continue;
            }
            if let Some(name) = line.strip_prefix(";TYPE:") {
                on_tower = name == "Prime tower";
                continue;
            }
            if let Some(name) = line.strip_prefix("; FEATURE:") {
                on_tower = name.trim() == "Prime tower";
                continue;
            }
            let code = line.split(';').next().unwrap_or("");
            let mut words = code.split_whitespace();
            match words.next() {
                Some("G90") => relative = false,
                Some("G91") => relative = true,
                Some("G0" | "G1" | "G2" | "G3") => {
                    let word = |letter: char| {
                        code.split_whitespace()
                            .skip(1)
                            .find_map(|word| word.strip_prefix(letter)?.parse::<f64>().ok())
                    };
                    let axis = |letter: char, now: f64| match word(letter) {
                        Some(v) if relative => now + v,
                        Some(v) => v,
                        None => now,
                    };
                    let (from, was) = (at, z);
                    at = [axis('X', at[0]), axis('Y', at[1])];
                    z = axis('Z', z);
                    if pass == 0 {
                        if on_tower && word('E').is_some_and(|e| e > 0.0) && at != from {
                            for point in [from, at] {
                                tower.0 = [tower.0[0].min(point[0]), tower.0[1].min(point[1])];
                                tower.1 = [tower.1[0].max(point[0]), tower.1[1].max(point[1])];
                            }
                        }
                        continue;
                    }
                    // The start against the layer it was on, the end
                    // against the one it reached.
                    let before = top;
                    if let Some(new) = coming.filter(|new| z >= new - 1e-3) {
                        top = Some(new);
                        coming = None;
                    }
                    let Some(top) = top else { continue };
                    if before.is_none_or(|before| was >= before - 1e-3) && z >= top - 1e-3 {
                        continue;
                    }
                    let inside = |point: [f64; 2]| {
                        (0..2).all(|axis| {
                            point[axis] >= tower.0[axis] - margin
                                && point[axis] <= tower.1[axis] + margin
                        })
                    };
                    if inside(from) && inside(at) {
                        lows.on_tower += 1;
                    } else {
                        lows.off_tower.push(format!(
                            "{line} (from {from:?} to {at:?} at {z} on the layer up to {top})"
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    lows
}

/// The reference's own file before aceRage/EdgeSlicer#1020, at the first
/// of its tool changes in the middle of a layer (`halves.stl`, 0.3 mm
/// layers, a multi-tool Klipper printer, no ramming, "Tool change on wipe
/// tower" off, no sparse layers; work/cpp/dump_main/in_place_5.gcode, the
/// lines from the layer at 3.3 mm to the end of its tower block, with the
/// progress, width and speed limit lines and most of the tower's brim
/// left out). `append_tcr2` took the nozzle down to the tower's 0.3 mm
/// at the end of the post's wall, X49.775 Y49.985, pushed the filament
/// back there, and went up again for the change.
const REFERENCE_DESCENT_OVER_THE_PART: &str = r#";Z:3.3
G92 E0
;_SET_FAN_SPEED_CHANGING_LAYER
; printing object halves.stl id:0 copy 0
G1 E-.8 F1800 ; retract
G1 X49.827 Y52.145 Z3.7 F7200 ; move to first perimeter point
G1 X49.775 Y50.025 Z3.7 ; move to first perimeter point
G1 Z3.3 F720 ; restore layer Z
G1 E.8 F1800 ;  ; unretract
;TYPE:Outer wall
G1 F1037.293
G1 X49.775 Y59.775 E.46894 ; perimeter
G1 X40.225 Y59.775 E.45932 ; perimeter
G1 X40.225 Y40.225 E.94029 ; perimeter
G1 X49.775 Y40.225 E.45932 ; perimeter
G1 X49.775 Y49.985 E.46942 ; perimeter
; stop printing object halves.stl id:0 copy 0
G1 E-.8 F1800 ; retract
G1 Z.3 F720 ; Travel down to the last wipe tower layer.
G1 E.8 F1800 ;  ; unretract
;TYPE:Prime tower
;--------------------
; CP TOOLCHANGE START
; toolchange #1
; material : PLA -> PLA
;--------------------
M220 S100
; WIPE_TOWER_START
; CP TOOLCHANGE UNLOAD
M400
G1 E-.8 ; retract
G1 Z3.3 F720 ; Restore layer Z before toolchange to avoid collision with printed parts
G1 E.8 F1800 ;  ; unretract
G1 E-10 ; retract for toolchange
T1 ; change extruder
G1 E-.8 ; retract
G1 Z3.7 F720 ; normal lift Z
G1 X159.774 Y134.951 Z3.7 F7200 ; Travel to a Wipe Tower
G1 X175.049 Y134.951
G1 X175.049 Y146.75
G1 X169.75 Y146.75
G1  X169.750 Y146.750
G1  X169.250 Y146.250
G1 Z.3 F720 ; Force restore layer Z
G1 F7200
G1 E.8 F1800 ;  ; unretract
G1  Y146.750
; CP TOOLCHANGE WIPE
G1  X166.250  E0.1630 F594
G1 E-0.8000 F1800
G1  X170.750  F600
G1  X169.250  F240
G1  X168.750  F600
G1  Y146.250
G1  X169.750
G1  Y147.250
G1  X168.250
G1  Y145.750
G1  X170.250
G1  Y147.750
G1  X166.250 Y146.750   F594
G1 E0.8000 F1800
G1  X173.049 Y136.951
G1  Y150.049  E0.7117
G1  X136.951  E1.9613
G1  Y136.951  E0.7117
G1  X173.049  E1.9613
; WIPE_TOWER_END
G1 E-.8 ; retract
G1 Z3.3 F720 ; Travel back up to the topmost object layer.
G1 E.8 F1800 ;  ; unretract
"#;

#[test]
fn the_check_finds_the_reference_going_down_over_the_part() {
    let lows = low_off_the_tower(REFERENCE_DESCENT_OVER_THE_PART, 1.0);
    // Down at the post, the filament pushed back and pulled again there,
    // and up from it for the change.
    assert_eq!(lows.off_tower.len(), 4, "{:#?}", lows.off_tower);
    assert!(
        lows.off_tower[0].starts_with("G1 Z.3 F720 ; Travel down to the last wipe tower layer.")
    );
    assert!(lows.off_tower[1].starts_with("G1 E.8 F1800"));
    assert!(lows.off_tower[3].contains("Restore layer Z before toolchange"));
    // What it then does on the tower is not taken for a fault.
    assert!(lows.on_tower > 10);
}

/// Every printer and tower this engine lays a tower for without sparse
/// layers, with the filament changing first thing on a layer (`bands`)
/// and in the middle of one, the nozzle over the post (`halves`): the
/// nozzle is never lower than the layer anywhere but over the tower, not
/// to travel, lay a line or move the filament alone. On the reference
/// before aceRage/EdgeSlicer#1020 the in-place change (a multi-tool
/// printer, no ramming, "Tool change on wipe tower" off) failed this.
#[test]
fn the_nozzle_is_below_the_layer_only_over_the_tower() {
    let dir = tempfile::tempdir().unwrap();
    let u1 = u1();
    let x1c = printer(
        "BBL",
        "Bambu Lab X1 Carbon 0.4 nozzle",
        "0.20mm Standard @BBL X1C",
        "Bambu PLA Basic @BBL X1C",
    );
    let cases: [(&str, &Chosen, &str); 7] = [
        // A multi-tool printer that rams the filament put away on the
        // tower, as the U1 is shipped.
        ("rammed", &u1, r#""tool_change_on_wipe_tower": "0""#),
        (
            "rammed, change on the tower",
            &u1,
            r#""tool_change_on_wipe_tower": "1""#,
        ),
        // No ramming: the reference changed tool where the nozzle was.
        (
            "in place",
            &u1,
            r#""filament_multitool_ramming": ["0", "0"], "tool_change_on_wipe_tower": "0""#,
        ),
        (
            "not rammed, change on the tower",
            &u1,
            r#""filament_multitool_ramming": ["0", "0"], "tool_change_on_wipe_tower": "1""#,
        ),
        // One nozzle, the reference's WipeTower2.
        (
            "one nozzle",
            &u1,
            r#""single_extruder_multi_material": "1", "enable_filament_ramming": "0""#,
        ),
        // Bambu's tower on a multi-tool printer and on a Bambu Lab one.
        ("Bambu tower", &u1, r#""wipe_tower_type": "type1""#),
        ("Bambu Lab X1 Carbon", &x1c, ""),
    ];
    let models = [("bands", bands(dir.path())), ("halves", halves(dir.path()))];
    for (name, chosen, set) in cases {
        let set = if set.is_empty() {
            ON.to_owned()
        } else {
            format!(r#"{{"wipe_tower_no_sparse_layers": "1", {set}}}"#)
        };
        for (shape, model) in &models {
            let gcode = slice_on(model, chosen, &set).unwrap();
            let lows = low_off_the_tower(&gcode, 1.0);
            assert!(
                lows.on_tower > 100,
                "{name}, {shape}: the tower is laid low"
            );
            assert!(
                lows.off_tower.is_empty(),
                "{name}, {shape}: {} lines below the layer off the tower: {:#?}",
                lows.off_tower.len(),
                &lows.off_tower[..lows.off_tower.len().min(10)]
            );
        }
    }
}
