use std::path::Path;

use super::*;

const SMALL: &str = "tests/data/support_corpus/twopart_groups.3mf";
const THREE_PLATES: &str = "tests/mcbrim_test.3mf";
const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
const MACHINE: &str = "Snapmaker U1 (0.4 nozzle)";

fn reference(path: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../reference/EdgeSlicer")
        .join(path)
        .to_string_lossy()
        .into_owned()
}

fn ask(session: &Arc<Session>, name: &str, args: Value) -> Value {
    session
        .call(name, &args)
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

fn bytes(session: &Arc<Session>, name: &str, args: Value) -> Vec<u8> {
    session
        .call_bytes(name, &args)
        .unwrap_or_else(|error| panic!("{name} {args}: {error}"))
}

fn open(session: &Arc<Session>, path: &str) -> u64 {
    ask(session, "project_open", json!({ "path": reference(path) }))["id"]
        .as_u64()
        .unwrap()
}

fn sliced(session: &Arc<Session>, project: u64) {
    ask(session, "slice", json!({ "project": project }));
    loop {
        let status = ask(session, "status", json!({ "project": project }));
        if status["state"] != "slicing" {
            assert_eq!(status["state"], "done", "{status}");
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn numbers(value: &Value) -> Vec<f64> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is not a list"))
        .iter()
        .map(|number| number.as_f64().unwrap())
        .collect()
}

/// A mesh as `mesh_bytes` lays it out: vertices, triangles, marks.
fn read_mesh(bytes: &[u8]) -> (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<u8>) {
    assert_eq!(&bytes[0..4], b"EDGM");
    assert_eq!(u32_at(bytes, 4), 1, "version");
    let (v, t) = (u32_at(bytes, 8) as usize, u32_at(bytes, 12) as usize);
    let (flags, uniform) = (u32_at(bytes, 16), u32_at(bytes, 20));
    let vertices = (0..v)
        .map(|at| std::array::from_fn(|axis| f32_at(bytes, 24 + 12 * at + 4 * axis)))
        .collect();
    let from = 24 + 12 * v;
    let triangles = (0..t)
        .map(|at| std::array::from_fn(|corner| u32_at(bytes, from + 12 * at + 4 * corner)))
        .collect();
    let from = from + 12 * t;
    let marks = if flags & 1 == 1 {
        assert_eq!(uniform, 0);
        assert_eq!(bytes.len(), (from + t).next_multiple_of(4));
        assert!(bytes[from + t..].iter().all(|pad| *pad == 0));
        bytes[from..from + t].to_vec()
    } else {
        assert_eq!(bytes.len(), from);
        vec![uniform as u8; t]
    };
    (vertices, triangles, marks)
}

#[test]
fn a_mesh_is_laid_out_byte_for_byte_as_written() {
    let object = SceneObject {
        name: "two".into(),
        vertices: vec![
            [0.0, 0.0, 0.0],
            [1.5, 0.0, 0.0],
            [0.0, -2.0, 0.0],
            [0.0, 0.0, 3.25],
        ],
        triangles: vec![[0, 1, 2], [0, 3, 1], [0, 2, 3]],
        marks: vec![
            Mark::Filament(1),
            Mark::Filament(255),
            Mark::SupportEnforcer,
        ],
        bounds: None,
    };
    let bytes = mesh_bytes(&object);
    #[rustfmt::skip]
    let expected: Vec<u8> = [
        &b"EDGM"[..],
        &[1, 0, 0, 0], &[4, 0, 0, 0], &[3, 0, 0, 0], &[1, 0, 0, 0], &[0, 0, 0, 0],
        // 1.5 is 0x3fc00000, -2 is 0xc0000000, 3.25 is 0x40500000.
        &[0; 12],
        &[0, 0, 0xc0, 0x3f], &[0; 8],
        &[0; 4], &[0, 0, 0, 0xc0], &[0; 4],
        &[0; 8], &[0, 0, 0x50, 0x40],
        &[0, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0],
        &[0, 0, 0, 0, 3, 0, 0, 0, 1, 0, 0, 0],
        &[0, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0],
        // A filament past 250 is given as 250; then a support enforcer; a zero to fill.
        &[1, 250, 254, 0],
    ]
    .concat();
    assert_eq!(bytes, expected);
    assert_eq!(bytes.len(), mesh_size(&object));

    // Every triangle alike: the mark is said once, and none follow.
    let alike = SceneObject {
        marks: vec![Mark::Filament(4); 3],
        ..object
    };
    let bytes = mesh_bytes(&alike);
    assert_eq!(bytes.len(), 24 + 48 + 36);
    assert_eq!((u32_at(&bytes, 16), u32_at(&bytes, 20)), (0, 4));
    assert_eq!(bytes.len(), mesh_size(&alike));
    assert_eq!(read_mesh(&bytes).2, [4, 4, 4]);
}

#[test]
fn a_move_is_laid_out_byte_for_byte_as_written() {
    let step = edge_engine::Move {
        kind: MoveKind::Extrude,
        feature: Feature::OuterWall,
        to: [1.5, -2.0, 0.25],
        width: 0.42,
        height: 0.2,
        feedrate: 100.0,
        fan: 80,
        temperature: 220,
        filament: 3,
        layer: 0x0102_0304,
        turn: -std::f32::consts::FRAC_PI_2,
    };
    let mut record = Vec::new();
    move_bytes(&mut record, &step);
    #[rustfmt::skip]
    let expected: Vec<u8> = [
        // x 1.5, y -2, z 0.25 (0x3e800000), then 100 mm/s (0x42c80000).
        &[0, 0, 0xc0, 0x3f][..], &[0, 0, 0, 0xc0], &[0, 0, 0x80, 0x3e], &[0, 0, 0xc8, 0x42],
        // Height 200 and width 420 thousandths; a quarter turn to the
        // right, -15708 ten-thousandths (0xc2a4); 220 degrees.
        &[200, 0], &[0xa4, 0x01], &[0xa4, 0xc2], &[220, 0],
        &[4, 3, 2, 1],
        // A line laid, an outer wall, filament 3, fan at 80%.
        &[2, 2, 3, 80],
    ]
    .concat();
    assert_eq!(record, expected);
    assert_eq!(record.len(), MOVE_SIZE);

    // A stretch says where it starts, how many it holds and of how many.
    let records: Vec<u8> = (0..5u8).flat_map(|at| [at; MOVE_SIZE]).collect();
    let stretch = moves_bytes(&records, 1, 3);
    assert_eq!(&stretch[0..4], b"EDGT");
    assert_eq!(
        [4, 8, 12, 16, 20].map(|at| u32_at(&stretch, at)),
        [1, 1, 3, 5, 32]
    );
    assert_eq!(stretch.len(), MOVES_HEADER + 3 * MOVE_SIZE);
    assert_eq!(stretch[MOVES_HEADER], 1);
    assert_eq!(stretch[stretch.len() - 1], 3);
}

#[test]
fn a_project_is_a_scene_of_plates_and_objects() {
    let session = Arc::new(Session::new(None));
    // A window's first project is empty: a scene with nothing in it.
    let empty = ask(&session, "scene", Value::Null);
    assert_eq!(empty["objects"], json!([]));
    assert_eq!(empty["placings"], json!([]));
    assert_eq!(empty["plates"], json!([]));
    assert_eq!(empty["bed"]["box"], Value::Null);
    assert!(
        session
            .call_bytes("scene_mesh", &json!({ "object": 0 }))
            .unwrap_err()
            .contains("empty")
    );

    let id = open(&session, THREE_PLATES);
    let scene = ask(&session, "scene", Value::Null);
    assert_eq!(scene["project"], id);
    assert_eq!(scene["bare"], false);
    assert_eq!(scene["plate"], 1);
    // The project's own printer: a U1's bed, 270 mm each way, and the
    // plates a fifth of a bed apart, the third starting a row towards -Y.
    assert_eq!(scene["bed"]["box"], json!([0.5, 1.0, 270.5, 271.0]));
    assert_eq!(scene["bed"]["shape"].as_array().unwrap().len(), 4);
    assert_eq!(scene["bed"]["height"], 270.05);
    let plates = scene["plates"].as_array().unwrap();
    let origins: Vec<Vec<f64>> = plates
        .iter()
        .map(|plate| numbers(&plate["origin"]))
        .collect();
    assert_eq!(origins, [[0.0, 0.0], [324.0, 0.0], [0.0, -324.0]]);
    assert_eq!(plates[2]["id"], 3);
    assert!(
        scene["colours"]
            .as_array()
            .is_some_and(|colours| colours.len() >= 7)
    );

    // Every copy stands within its own plate's bed, on the bed.
    let objects = scene["objects"].as_array().unwrap();
    let placings = scene["placings"].as_array().unwrap();
    assert!(!objects.is_empty() && placings.len() >= objects.len());
    for placing in placings {
        let plate = placing["plate"].as_u64().expect("on a plate") as usize;
        let origin = &origins[plate - 1];
        let (low, high) = (numbers(&placing["box"][0]), numbers(&placing["box"][1]));
        assert!(
            low[0] >= origin[0] && high[0] <= origin[0] + 271.0,
            "{placing}"
        );
        assert!(
            low[1] >= origin[1] && high[1] <= origin[1] + 271.5,
            "{placing}"
        );
        assert!(low[2].abs() < 0.5 && high[2] > low[2], "{placing}");
        let matrix = numbers(&placing["matrix"]);
        assert_eq!(matrix.len(), 16);
        assert_eq!(
            [matrix[3], matrix[7], matrix[11], matrix[15]],
            [0.0, 0.0, 0.0, 1.0]
        );
        assert!(placing["object"].as_u64().unwrap() < objects.len() as u64);
    }
    // What each plate holds is what the project lists on it.
    let project = ask(&session, "project", Value::Null);
    for plate in project["plates"].as_array().unwrap() {
        let on_it = placings
            .iter()
            .filter(|placing| placing["plate"] == plate["id"] && placing["printable"] == true)
            .count();
        assert_eq!(on_it, plate["objects"].as_array().unwrap().len(), "{plate}");
    }

    // Each object's mesh, as many bytes as the scene said, every corner
    // a vertex of it, every mark a filament the object is said to use.
    let mut painted = 0;
    for (index, object) in objects.iter().enumerate() {
        let mesh = bytes(&session, "scene_mesh", json!({ "object": index }));
        assert_eq!(mesh.len() as u64, object["bytes"].as_u64().unwrap());
        let (vertices, triangles, marks) = read_mesh(&mesh);
        assert_eq!(vertices.len() as u64, object["vertices"].as_u64().unwrap());
        assert_eq!(
            triangles.len() as u64,
            object["triangles"].as_u64().unwrap()
        );
        assert!(
            triangles
                .iter()
                .flatten()
                .all(|at| (*at as usize) < vertices.len())
        );
        let used: Vec<u64> = object["filaments"]
            .as_array()
            .unwrap()
            .iter()
            .map(|filament| filament.as_u64().unwrap())
            .collect();
        assert!(
            marks.iter().all(|mark| used.contains(&u64::from(*mark))),
            "{object}"
        );
        painted += usize::from(used.len() > 1);
        // The same bytes every time.
        assert_eq!(
            mesh,
            bytes(&session, "scene_mesh", json!({ "object": index }))
        );
    }
    assert!(painted > 0, "the project has painted objects");
    let past = session
        .call_bytes("scene_mesh", &json!({ "object": objects.len() }))
        .unwrap_err();
    assert!(past.contains("there is no object"), "{past}");

    // The scene is the same until the file is another; going to another
    // plate changes which is shown, not the scene.
    ask(&session, "project_set", json!({ "plate": 2 }));
    let again = ask(&session, "scene", Value::Null);
    assert_eq!(again["key"], scene["key"]);
    assert_eq!(again["plate"], 2);
    assert_eq!(again["placings"], scene["placings"]);

    // A mesh is bytes and a scene is JSON; neither door opens the other.
    let wrong = session
        .call("scene_mesh", &json!({ "object": 0 }))
        .unwrap_err();
    assert!(wrong.contains("bytes"), "{wrong}");
    let wrong = session.call_bytes("scene", &Value::Null).unwrap_err();
    assert!(wrong.contains("no call named scene"), "{wrong}");
    assert!(session.call_bytes("nonsense", &Value::Null).is_err());
}

#[test]
fn the_bed_is_drawn_from_the_vendors_own_files() {
    let session = Arc::new(Session::new(None));
    open(&session, SMALL);
    // The project was saved for a Snapmaker U1, whose profile has a model
    // of its bed.
    let scene = ask(&session, "scene", Value::Null);
    assert_eq!(scene["bed"]["model"], "Snapmaker U1_bed.stl");
    let model = bytes(&session, "bed_model", Value::Null);
    let (vertices, triangles, marks) = read_mesh(&model);
    assert!(vertices.len() > 8 && triangles.len() > 12);
    assert!(marks.iter().all(|mark| *mark == 1));
    // Its origin is the middle of the bed: it reaches about as far each way.
    let reach = |axis: usize| {
        let values = vertices.iter().map(|vertex| vertex[axis]);
        (
            values.clone().fold(f32::INFINITY, f32::min),
            values.fold(f32::NEG_INFINITY, f32::max),
        )
    };
    let (left, right) = reach(0);
    assert!(left < -100.0 && right > 100.0, "{left} {right}");

    // Another printer, another bed: an A350's has a picture too.
    let chosen = ask(
        &session,
        "project_set",
        json!({ "source": "presets", "vendor": "Snapmaker", "machine": "Snapmaker A350 (0.4 nozzle)" }),
    );
    assert_eq!(chosen["choices"]["machine"], "Snapmaker A350 (0.4 nozzle)");
    let scene = ask(&session, "scene", Value::Null);
    assert_eq!(scene["bed"]["model"], "Snapmaker A350_bed.stl");
    assert_eq!(scene["bed"]["texture"], "Snapmaker A350_texture.svg");
    let picture = bytes(&session, "bed_texture", Value::Null);
    assert!(String::from_utf8_lossy(&picture).contains("<svg"));

    // No printer, no bed to draw; the calls say so.
    let session = Arc::new(Session::new(None));
    let none = session.call_bytes("bed_model", &Value::Null).unwrap_err();
    assert!(none.contains("no model of its bed"), "{none}");
    assert!(session.call_bytes("bed_texture", &Value::Null).is_err());
}

#[test]
fn a_bare_model_stands_in_the_middle_of_the_bed() {
    let session = Arc::new(Session::new(None));
    let id = open(&session, BARE);
    // Before a printer is chosen there is no bed: it stands on the ground.
    let scene = ask(&session, "scene", Value::Null);
    assert_eq!(scene["bare"], true);
    assert_eq!(scene["plates"], json!([]));
    assert_eq!(scene["bed"]["box"], Value::Null);
    assert_eq!(scene["placings"][0]["plate"], Value::Null);
    assert!(numbers(&scene["placings"][0]["box"][0])[2].abs() < 1e-6);
    let key = scene["key"].clone();

    ask(
        &session,
        "project_set",
        json!({ "project": id, "source": "presets", "vendor": "Snapmaker", "machine": MACHINE }),
    );
    let scene = ask(&session, "scene", Value::Null);
    let bed = numbers(&scene["bed"]["box"]);
    let (low, high) = (
        numbers(&scene["placings"][0]["box"][0]),
        numbers(&scene["placings"][0]["box"][1]),
    );
    for axis in 0..2 {
        let middle = (bed[axis] + bed[axis + 2]) / 2.0;
        assert!(
            ((low[axis] + high[axis]) / 2.0 - middle).abs() < 2e-3,
            "{low:?} {high:?}"
        );
    }
    assert!(low[2].abs() < 1e-6 && high[2] > 1.0);
    // Where it stands is where its outline was shown from above.
    let above = ask(&session, "plate_view", Value::Null);
    let outline = numbers(&above["objects"][0]["box"]);
    assert!((outline[0] - low[0]).abs() < 2e-3 && (outline[3] - high[1]).abs() < 2e-3);
    // The printer changed, the file did not: the mesh is not asked for again.
    assert_eq!(scene["key"], key);
    assert_eq!(scene["objects"][0]["filaments"], json!([1]));
}

#[test]
fn a_sliced_plate_is_given_as_moves_a_stretch_at_a_time() {
    let session = Arc::new(Session::new(None));
    let id = open(&session, SMALL);
    let early = session.call("toolpath", &Value::Null).unwrap_err();
    assert!(early.contains("nothing has been sliced"), "{early}");
    assert!(
        session
            .call_bytes("toolpath_moves", &json!({ "from": 0, "count": 10 }))
            .is_err()
    );
    sliced(&session, id);

    let index = ask(&session, "toolpath", Value::Null);
    let summary = ask(&session, "summary", Value::Null);
    assert_eq!(index["source"], "gcode");
    assert_eq!(index["run"], 1);
    assert_eq!(index["move_size"], 32);
    let total = index["moves"].as_u64().unwrap() as usize;
    let first = numbers(&index["layers"]["first"]);
    assert_eq!(first.len() as u64, summary["layers"].as_u64().unwrap());
    assert_eq!(first[0], 0.0);
    assert!(first.windows(2).all(|pair| pair[0] < pair[1]) && (first[59] as usize) < total);
    assert_eq!(numbers(&index["layers"]["z"])[0], 0.25);
    assert_eq!(numbers(&index["layers"]["height"])[1], 0.2);
    // The features by the reference's numbers: inner wall 1, outer wall
    // 2, the file's own (the purge line) 14.
    let features = numbers(&index["features"]);
    assert!(
        [1.0, 2.0, 14.0]
            .iter()
            .all(|feature| features.contains(feature)),
        "{features:?}"
    );
    assert_eq!(index["filaments"], json!([1]));
    let counts = &index["counts"];
    let counted: u64 = [
        "travel",
        "line",
        "wipe",
        "retract",
        "unretract",
        "seam",
        "tool_change",
    ]
    .iter()
    .map(|kind| counts[kind].as_u64().unwrap())
    .sum();
    assert!(counts["seam"].as_u64().unwrap() >= 57 && counts["travel"].as_u64().unwrap() > 100);
    assert!(counted < total as u64, "the starts are moves too");
    // The part, not the purge line at the edge of the bed.
    let (low, high) = (numbers(&index["box"][0]), numbers(&index["box"][1]));
    assert!(
        high[0] - low[0] < 40.0 && high[2] > 12.0 && high[2] < 12.2,
        "{low:?} {high:?}"
    );

    // In stretches: each says where it is, and together they are all of it.
    let mut all = Vec::new();
    let mut from = 0;
    while from < total {
        let stretch = bytes(
            &session,
            "toolpath_moves",
            json!({ "from": from, "count": 1000 }),
        );
        assert_eq!(&stretch[0..4], b"EDGT");
        let count = u32_at(&stretch, 12) as usize;
        assert_eq!(
            [4, 8, 16, 20].map(|at| u32_at(&stretch, at) as usize),
            [1, from, total, 32]
        );
        assert_eq!(count, 1000.min(total - from));
        assert_eq!(stretch.len(), MOVES_HEADER + count * MOVE_SIZE);
        all.extend_from_slice(&stretch[MOVES_HEADER..]);
        from += count;
    }
    let whole = bytes(
        &session,
        "toolpath_moves",
        json!({ "from": 0, "count": total }),
    );
    assert_eq!(&whole[MOVES_HEADER..], &all[..]);
    // The same bytes when asked again.
    assert_eq!(
        whole,
        bytes(
            &session,
            "toolpath_moves",
            json!({ "from": 0, "count": total })
        )
    );
    // The first layer starts where the file does; every other with a
    // line laid on it, at its height.
    assert_eq!(
        all[28], 0,
        "the first move is where the drawing starts from"
    );
    for (layer, start) in first.iter().enumerate().skip(1) {
        let record = &all[*start as usize * MOVE_SIZE..][..MOVE_SIZE];
        assert_eq!(record[28], 2, "layer {layer} starts with a line");
        assert_eq!(u32_at(record, 24) as usize, layer);
        let z = numbers(&index["layers"]["z"])[layer];
        assert!((f64::from(f32_at(record, 8)) - z).abs() < 1e-3);
    }
    // Nothing past the end; the end itself is an empty stretch.
    let end = bytes(
        &session,
        "toolpath_moves",
        json!({ "from": total, "count": 5 }),
    );
    assert_eq!((end.len(), u32_at(&end, 12)), (MOVES_HEADER, 0));
    let past = session
        .call_bytes("toolpath_moves", &json!({ "from": total + 1, "count": 5 }))
        .unwrap_err();
    assert!(past.contains("there is no move"), "{past}");
    assert!(
        session
            .call_bytes("toolpath_moves", &json!({ "from": 0 }))
            .is_err()
    );

    // Another slice is another run: the page asks again.
    ask(&session, "project_set", json!({ "wall_loops": 3 }));
    assert!(session.call("toolpath", &Value::Null).is_err());
    sliced(&session, id);
    let again = ask(&session, "toolpath", Value::Null);
    assert_eq!(again["run"], 2);
    assert_ne!(again["moves"], index["moves"]);
}

#[test]
fn without_gcode_the_lines_are_still_shown() {
    let session = Arc::new(Session::new(None));
    let id = open(&session, BARE);
    // A printer with no filament preset for the model: sliced, but no
    // G-code can be written.
    ask(
        &session,
        "project_set",
        json!({ "project": id, "source": "presets", "vendor": "Snapmaker", "machine": MACHINE }),
    );
    ask(&session, "slice", json!({ "project": id }));
    let status = loop {
        let status = ask(&session, "status", json!({ "project": id }));
        if status["state"] != "slicing" {
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    if status["state"] != "done" {
        // Nothing sliced, nothing to show: the call says so.
        assert!(session.call("toolpath", &Value::Null).is_err());
        return;
    }
    let summary = ask(&session, "summary", Value::Null);
    let index = ask(&session, "toolpath", Value::Null);
    let expected = if summary["gcode"].get("error").is_some() {
        "lines"
    } else {
        "gcode"
    };
    assert_eq!(index["source"], expected);
    assert_eq!(
        index["layers"]["first"].as_array().unwrap().len() as u64,
        summary["layers"].as_u64().unwrap()
    );
}
