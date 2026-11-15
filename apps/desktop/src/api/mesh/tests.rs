use std::path::Path;

use super::*;

const BARE: &str = "resources/handy_models/OrcaToleranceTest.stl";
const BENCHY: &str = "resources/handy_models/3DBenchy.3mf";

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

fn open(session: &Arc<Session>, path: &str) -> u64 {
    ask(session, "project_open", json!({ "path": reference(path) }))["id"]
        .as_u64()
        .unwrap()
}

#[test]
fn a_simplified_part_is_shown_applied_as_one_step_and_undone() {
    let session = Arc::new(Session::new(None));
    open(&session, BARE);
    let scene = ask(&session, "scene", json!({}));
    let object = &scene["objects"][0];
    let id = object["id"].clone();
    let before = object["triangles"].as_u64().unwrap();
    // A closed mesh read as it is: no warning.
    assert_eq!(object["errors"], Value::Null);
    assert_eq!(object["part_list"][0]["errors"], Value::Null);

    // "Detail level" "Low" (0.5).
    let made = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "max_error": 0.5 }),
    );
    assert_eq!(made["original"].as_u64(), Some(before));
    let after = made["triangles"].as_u64().unwrap();
    assert!(after < before, "{after} of {before}");
    let progress = ask(&session, "mesh_progress", json!({}));
    assert_eq!(progress["running"], false);
    assert_eq!(progress["id"], made["id"]);
    let bytes = session
        .call_bytes("mesh_result", &json!({ "id": made["id"] }))
        .unwrap();
    assert!(!bytes.is_empty());

    // "Decimate ratio" 50 %: the later one takes the place of the first.
    let half = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "wanted": before / 2 }),
    );
    assert!(half["triangles"].as_u64().unwrap() <= before / 2);
    assert!(
        session
            .call("mesh_apply", &json!({ "id": made["id"] }))
            .is_err()
    );

    let applied = ask(&session, "mesh_apply", json!({ "id": half["id"] }));
    assert_eq!(applied["done"], "Simplify OrcaToleranceTest.stl");
    assert_eq!(
        applied["project"]["edit"]["undo"],
        "Simplify OrcaToleranceTest.stl"
    );
    assert_eq!(applied["note"], Value::Null);
    assert_eq!(
        applied["scene"]["objects"][0]["triangles"],
        half["triangles"]
    );
    // Kept no longer: put in once.
    assert!(
        session
            .call("mesh_apply", &json!({ "id": half["id"] }))
            .is_err()
    );

    let undone = ask(&session, "undo", json!({}));
    assert_eq!(
        undone["scene"]["objects"][0]["triangles"].as_u64(),
        Some(before)
    );
}

#[test]
fn a_result_made_before_an_edit_is_not_put_in_after_it() {
    let session = Arc::new(Session::new(None));
    open(&session, BARE);
    let scene = ask(&session, "scene", json!({}));
    let id = scene["objects"][0]["id"].clone();
    let made = ask(
        &session,
        "mesh_simplify",
        json!({ "object": id, "max_error": 1.0 }),
    );
    ask(
        &session,
        "scene_edit",
        json!({ "op": "rename", "object": id, "name": "other" }),
    );
    let refused = session
        .call("mesh_apply", &json!({ "id": made["id"] }))
        .unwrap_err();
    assert!(refused.contains("changed meanwhile"), "{refused}");
}

#[test]
fn a_simplify_is_called_off() {
    let session = Arc::new(Session::new(None));
    open(&session, BENCHY);
    let scene = ask(&session, "scene", json!({}));
    let id = scene["objects"][0]["id"].clone();
    let worker = {
        let session = session.clone();
        let id = id.clone();
        std::thread::spawn(move || {
            session.call("mesh_simplify", &json!({ "object": id, "wanted": 0 }))
        })
    };
    // Called off once it is under way.
    loop {
        let progress = ask(&session, "mesh_progress", json!({}));
        if progress["running"] == true {
            ask(&session, "mesh_cancel", json!({}));
            break;
        }
        if worker.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert_eq!(worker.join().unwrap().unwrap_err(), "stopped");
    assert_eq!(ask(&session, "mesh_progress", json!({}))["running"], false);
}

/// A sphere of radius 8 about (0, 0, 8), with `holes` triangles taken out.
fn ball_stl(dir: &Path, holes: usize) -> String {
    let (rings, sectors) = (24usize, 48usize);
    let point = |ring: usize, sector: usize| -> [f32; 3] {
        if ring == 0 || ring == rings {
            return [0.0, 0.0, if ring == 0 { 16.0 } else { 0.0 }];
        }
        let theta = std::f64::consts::PI * ring as f64 / rings as f64;
        let phi = 2.0 * std::f64::consts::PI * (sector % sectors) as f64 / sectors as f64;
        [
            (8.0 * theta.sin() * phi.cos()) as f32,
            (8.0 * theta.sin() * phi.sin()) as f32,
            (8.0 + 8.0 * theta.cos()) as f32,
        ]
    };
    let mut soup = Vec::new();
    for ring in 0..rings {
        for sector in 0..sectors {
            let (a, b) = (point(ring, sector), point(ring, sector + 1));
            let (c, d) = (point(ring + 1, sector + 1), point(ring + 1, sector));
            if ring > 0 {
                soup.push([a, d, b]);
            }
            if ring + 1 < rings {
                soup.push([b, d, c]);
            }
        }
    }
    let mut mesh = edge_engine::Mesh::from_triangle_soup(soup);
    mesh.triangles.drain(500..500 + holes);
    let path = dir.join("ball.stl");
    std::fs::write(&path, binary_stl(&mesh)).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn a_part_with_a_hole_is_repaired_by_remeshing_as_one_step() {
    let dir = tempfile::tempdir().unwrap();
    let session = Arc::new(Session::new(None));
    ask(
        &session,
        "project_open",
        json!({ "path": ball_stl(dir.path(), 3) }),
    );
    let scene = ask(&session, "scene", json!({}));
    let object = &scene["objects"][0];
    let id = object["id"].clone();
    // The warning: the hole's rim is left open.
    assert_eq!(object["errors"]["open_edges"], 5, "{}", object["errors"]);
    assert_eq!(object["errors"]["repaired"], 0);

    let info = ask(&session, "mesh_repair_info", json!({ "object": id }));
    assert_eq!(info["parts"], 1);
    // The diagonal of a 16 mm box over 300.
    let auto = info["auto_voxel"].as_f64().unwrap();
    assert!((auto - 16.0 * 3f64.sqrt() / 300.0).abs() < 1e-6, "{auto}");
    assert!(info["area"].as_f64().unwrap() > 780.0);

    let answer = ask(
        &session,
        "mesh_repair",
        json!({ "object": id, "options": { "voxel_size": auto, "keep_bottom_flat": true, "preserve_sharp_edges": true, "sharp_feature_angle": 45.0 } }),
    );
    assert_eq!(answer["done"], "Repair by remeshing");
    assert_eq!(answer["project"]["edit"]["undo"], "Repair by remeshing");
    let note = answer["note"].as_str().unwrap();
    assert!(
        note.starts_with("Repaired 1 part(s) by remeshing. Triangles: "),
        "{note}"
    );
    // A round bottom: no flat base to keep.
    assert!(
        note.ends_with("1 part(s) had no flat bottom to protect."),
        "{note}"
    );
    let now = &answer["scene"]["objects"][0];
    assert_eq!(now["errors"], Value::Null, "{}", now["errors"]);
    assert_eq!(ask(&session, "mesh_progress", json!({}))["running"], false);

    let undone = ask(&session, "undo", json!({}));
    assert_eq!(undone["scene"]["objects"][0]["errors"]["open_edges"], 5);
}

fn binary_stl(mesh: &edge_engine::Mesh) -> Vec<u8> {
    let mut out = vec![0u8; 80];
    out.extend((mesh.triangles.len() as u32).to_le_bytes());
    for t in &mesh.triangles {
        out.extend([0u8; 12]);
        for &v in t {
            for c in mesh.vertices[v as usize] {
                out.extend(c.to_le_bytes());
            }
        }
        out.extend([0u8; 2]);
    }
    out
}
